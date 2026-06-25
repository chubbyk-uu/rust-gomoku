//! Android JNI bridge for the shared game controller.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jlong, jstring};
use jni::JNIEnv;
use rust_gomoku::{
    load_default_config, EngineProfile, GameController, RuleSet, SearchDifficulty, BLACK,
    BOARD_SIZE, WHITE,
};
use serde::Deserialize;
use serde_json::{json, Value};

type SharedController = Arc<Mutex<GameController>>;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

static REGISTRY: OnceLock<BridgeRegistry> = OnceLock::new();

#[derive(Default)]
struct BridgeRegistry {
    next_handle: AtomicI64,
    controllers: Mutex<HashMap<i64, SharedController>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum BridgeRequest {
    State,
    NewGame { human_side: String, rule: String },
    Play { x: usize, y: usize },
    Undo,
    SetProfile { profile: String },
    SetDifficulty { difficulty: String },
    EngineMove,
}

#[derive(Debug)]
struct BridgeError {
    code: &'static str,
    message: String,
}

impl BridgeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }

    fn invalid_handle() -> Self {
        Self::new(
            "invalid_handle",
            "Native game handle is invalid or destroyed.",
        )
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }
}

impl BridgeRegistry {
    fn create(&self) -> Result<i64, BridgeError> {
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed) + 1;
        let controller = Arc::new(Mutex::new(GameController::new(load_default_config())));
        self.controllers
            .lock()
            .map_err(|_| BridgeError::internal("Native handle registry is unavailable."))?
            .insert(handle, controller);
        Ok(handle)
    }

    fn destroy(&self, handle: i64) -> Result<bool, BridgeError> {
        Ok(self
            .controllers
            .lock()
            .map_err(|_| BridgeError::internal("Native handle registry is unavailable."))?
            .remove(&handle)
            .is_some())
    }

    fn controller(&self, handle: i64) -> Result<SharedController, BridgeError> {
        if handle <= 0 {
            return Err(BridgeError::invalid_handle());
        }
        self.controllers
            .lock()
            .map_err(|_| BridgeError::internal("Native handle registry is unavailable."))?
            .get(&handle)
            .cloned()
            .ok_or_else(BridgeError::invalid_handle)
    }

    fn request(&self, handle: i64, request_json: &str) -> Result<Value, BridgeError> {
        if request_json.len() > MAX_REQUEST_BYTES {
            return Err(BridgeError::invalid_request(format!(
                "Request JSON exceeds the {MAX_REQUEST_BYTES}-byte limit."
            )));
        }
        let request = parse_request(request_json)?;
        let controller = self.controller(handle)?;
        dispatch_request(controller, request)
    }
}

fn registry() -> &'static BridgeRegistry {
    REGISTRY.get_or_init(BridgeRegistry::default)
}

fn parse_request(request_json: &str) -> Result<BridgeRequest, BridgeError> {
    let value: Value = serde_json::from_str(request_json)
        .map_err(|err| BridgeError::invalid_request(format!("Invalid request JSON: {err}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| BridgeError::invalid_request("Request JSON must be an object."))?;
    let operation = object
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::invalid_request("Request field \"op\" must be a string."))?;
    let allowed_fields: &[&str] = match operation {
        "state" | "undo" | "engine_move" => &["op"],
        "new_game" => &["op", "human_side", "rule"],
        "play" => &["op", "x", "y"],
        "set_profile" => &["op", "profile"],
        "set_difficulty" => &["op", "difficulty"],
        _ => {
            return Err(BridgeError::invalid_request(format!(
                "Unknown operation \"{operation}\"."
            )))
        }
    };
    if let Some(field) = object
        .keys()
        .find(|field| !allowed_fields.contains(&field.as_str()))
    {
        return Err(BridgeError::invalid_request(format!(
            "Unknown field \"{field}\" for operation \"{operation}\"."
        )));
    }
    serde_json::from_value(value)
        .map_err(|err| BridgeError::invalid_request(format!("Invalid request fields: {err}")))
}

fn dispatch_request(
    controller: SharedController,
    request: BridgeRequest,
) -> Result<Value, BridgeError> {
    match request {
        BridgeRequest::State => snapshot_response(&controller),
        BridgeRequest::NewGame { human_side, rule } => {
            let side = parse_side(&human_side)?;
            let rule = parse_rule(&rule)?;
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .new_game(side, rule);
            snapshot_response(&controller)
        }
        BridgeRequest::Play { x, y } => {
            if x >= BOARD_SIZE || y >= BOARD_SIZE {
                return Err(BridgeError::invalid_request(format!(
                    "Coordinates ({x}, {y}) are outside the {BOARD_SIZE}x{BOARD_SIZE} board."
                )));
            }
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .play_human(x, y);
            snapshot_response(&controller)
        }
        BridgeRequest::Undo => {
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .undo_turn();
            snapshot_response(&controller)
        }
        BridgeRequest::SetProfile { profile } => {
            let profile = parse_profile(&profile)?;
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_profile(profile);
            snapshot_response(&controller)
        }
        BridgeRequest::SetDifficulty { difficulty } => {
            let difficulty = parse_difficulty(&difficulty)?;
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_difficulty(difficulty);
            snapshot_response(&controller)
        }
        BridgeRequest::EngineMove => {
            let task = controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .prepare_engine_search();
            let Some(task) = task else {
                return Err(BridgeError::invalid_request(
                    "The engine cannot move in the current position.",
                ));
            };
            let completion = task.run();
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .commit_engine_search(completion);
            snapshot_response(&controller)
        }
    }
}

fn snapshot_response(controller: &SharedController) -> Result<Value, BridgeError> {
    let snapshot = controller
        .lock()
        .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
        .snapshot();
    Ok(json!({ "ok": true, "state": snapshot }))
}

fn parse_side(value: &str) -> Result<i8, BridgeError> {
    match value {
        "black" => Ok(BLACK),
        "white" => Ok(WHITE),
        _ => Err(BridgeError::invalid_request(
            "human_side must be \"black\" or \"white\".",
        )),
    }
}

fn parse_rule(value: &str) -> Result<RuleSet, BridgeError> {
    value
        .parse()
        .map_err(|_| BridgeError::invalid_request("rule must be \"freestyle\" or \"renju\"."))
}

fn parse_profile(value: &str) -> Result<EngineProfile, BridgeError> {
    value
        .parse()
        .map_err(|_| BridgeError::invalid_request("profile must be \"base\" or \"fast\"."))
}

fn parse_difficulty(value: &str) -> Result<SearchDifficulty, BridgeError> {
    value.parse().map_err(|_| {
        BridgeError::invalid_request(
            "difficulty must be \"easy\", \"normal\", \"advanced\", or \"hard\".",
        )
    })
}

fn success_or_error(result: Result<Value, BridgeError>) -> String {
    let value = match result {
        Ok(value) => value,
        Err(err) => json!({
            "ok": false,
            "error": {
                "code": err.code,
                "message": err.message,
            }
        }),
    };
    serde_json::to_string(&value).unwrap_or_else(|_| {
        r#"{"ok":false,"error":{"code":"internal_error","message":"Response serialization failed."}}"#
            .to_string()
    })
}

fn panic_response() -> String {
    success_or_error(Err(BridgeError::internal(
        "Native request panicked and was contained.",
    )))
}

#[no_mangle]
pub extern "system" fn Java_io_github_chubbykuu_rustgomoku_NativeBridge_nativeCreate(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    catch_unwind(AssertUnwindSafe(|| registry().create().unwrap_or(0))).unwrap_or(0)
}

#[no_mangle]
pub extern "system" fn Java_io_github_chubbykuu_rustgomoku_NativeBridge_nativeRequest(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    request: JString,
) -> jstring {
    let response = catch_unwind(AssertUnwindSafe(|| {
        if request.is_null() {
            return success_or_error(Err(BridgeError::invalid_request(
                "Request JSON must not be null.",
            )));
        }
        match env.get_string(&request) {
            Ok(value) => success_or_error(registry().request(handle, &value.to_string_lossy())),
            Err(err) => success_or_error(Err(BridgeError::invalid_request(format!(
                "Request string could not be read: {err}"
            )))),
        }
    }))
    .unwrap_or_else(|_| panic_response());

    catch_unwind(AssertUnwindSafe(|| {
        env.new_string(response)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_io_github_chubbykuu_rustgomoku_NativeBridge_nativeDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| registry().destroy(handle)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::thread;

    fn response(registry: &BridgeRegistry, handle: i64, request: &str) -> Value {
        serde_json::from_str(&success_or_error(registry.request(handle, request))).unwrap()
    }

    #[test]
    fn state_and_mutation_requests_return_snapshots() {
        let registry = BridgeRegistry::default();
        let handle = registry.create().unwrap();

        let initial = response(&registry, handle, r#"{"op":"state"}"#);
        assert_eq!(initial["ok"], true);
        assert_eq!(initial["state"]["move_count"], 0);

        let new_game = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"black","rule":"renju"}"#,
        );
        assert_eq!(new_game["state"]["params"]["rule"], "renju");
        assert_eq!(new_game["state"]["human_side"], BLACK);

        let played = response(&registry, handle, r#"{"op":"play","x":7,"y":7}"#);
        assert_eq!(played["state"]["move_count"], 1);
        assert_eq!(played["state"]["side_to_move"], WHITE);

        let undone = response(&registry, handle, r#"{"op":"undo"}"#);
        assert_eq!(undone["state"]["move_count"], 0);

        let profile = response(
            &registry,
            handle,
            r#"{"op":"set_profile","profile":"fast"}"#,
        );
        assert_eq!(profile["state"]["params"]["profile"], "fast");

        let difficulty = response(
            &registry,
            handle,
            r#"{"op":"set_difficulty","difficulty":"advanced"}"#,
        );
        assert_eq!(difficulty["state"]["params"]["difficulty"], "advanced");
        assert_eq!(difficulty["state"]["params"]["depth"], 6);
        assert_eq!(difficulty["state"]["params"]["width"], 30);
        assert_eq!(difficulty["state"]["params"]["compute_vcf"], true);
        assert_eq!(difficulty["state"]["params"]["compute_vct"], true);
    }

    #[test]
    fn invalid_requests_have_stable_structured_errors() {
        let registry = BridgeRegistry::default();
        let handle = registry.create().unwrap();

        for (request, expected) in [
            ("not json", "invalid_request"),
            (r#"{"op":"unknown"}"#, "invalid_request"),
            (r#"{"op":"state","unexpected":true}"#, "invalid_request"),
            (
                r#"{"op":"new_game","human_side":"green","rule":"renju"}"#,
                "invalid_request",
            ),
            (
                r#"{"op":"new_game","human_side":"black","rule":"standard"}"#,
                "invalid_request",
            ),
            (r#"{"op":"play","x":15,"y":0}"#, "invalid_request"),
            (
                r#"{"op":"set_profile","profile":"turbo"}"#,
                "invalid_request",
            ),
            (
                r#"{"op":"set_difficulty","difficulty":"impossible"}"#,
                "invalid_request",
            ),
        ] {
            let value = response(&registry, handle, request);
            assert_eq!(value["ok"], false, "{request}");
            assert_eq!(value["error"]["code"], expected, "{request}");
        }

        let invalid_handle = response(&registry, handle + 1, r#"{"op":"state"}"#);
        assert_eq!(invalid_handle["error"]["code"], "invalid_handle");

        let oversized = " ".repeat(MAX_REQUEST_BYTES + 1);
        let value = response(&registry, handle, &oversized);
        assert_eq!(value["error"]["code"], "invalid_request");
    }

    #[test]
    fn destroy_invalidates_handle_and_is_idempotent() {
        let registry = BridgeRegistry::default();
        let handle = registry.create().unwrap();

        assert!(registry.destroy(handle).unwrap());
        assert!(!registry.destroy(handle).unwrap());
        let value = response(&registry, handle, r#"{"op":"state"}"#);
        assert_eq!(value["error"]["code"], "invalid_handle");
    }

    #[test]
    fn in_flight_request_keeps_controller_alive_after_destroy() {
        let registry = Arc::new(BridgeRegistry::default());
        let handle = registry.create().unwrap();
        let controller = registry.controller(handle).unwrap();
        let request_controller = Arc::clone(&controller);

        let worker = thread::spawn(move || {
            dispatch_request(request_controller, BridgeRequest::State).unwrap()
        });
        assert!(registry.destroy(handle).unwrap());

        let value = worker.join().unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["state"]["move_count"], 0);
        assert_eq!(Arc::strong_count(&controller), 1);
    }

    #[test]
    fn engine_move_runs_and_commits_through_dispatch() {
        let registry = BridgeRegistry::default();
        let handle = registry.create().unwrap();
        response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"white","rule":"renju"}"#,
        );

        let moved = response(&registry, handle, r#"{"op":"engine_move"}"#);
        assert_eq!(moved["ok"], true);
        assert_eq!(moved["state"]["move_count"], 1);
        assert_eq!(moved["state"]["moves"][0]["x"], BOARD_SIZE / 2);
        assert_eq!(moved["state"]["moves"][0]["y"], BOARD_SIZE / 2);
    }
}
