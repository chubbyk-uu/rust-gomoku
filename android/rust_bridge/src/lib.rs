//! Android JNI bridge for the shared game controller.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jlong, jstring};
use jni::JNIEnv;
use rust_gomoku::{
    default_interactive_threads, load_default_config, GameController, GameMode, RuleSet,
    SearchClockMode, SearchDifficulty, UiLanguage, BLACK, BOARD_SIZE, MAX_SEARCH_THREADS,
    SUPPORTED_TIME_LIMITS_MS, WHITE,
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
    NewGame {
        human_side: String,
        rule: String,
        mode: Option<String>,
        clock_mode: Option<String>,
    },
    Play {
        x: usize,
        y: usize,
    },
    Undo,
    SetDifficulty {
        difficulty: String,
    },
    SetTimeLimit {
        time_limit_ms: u32,
    },
    SetThreads {
        threads: usize,
    },
    SetMultithread {
        enabled: bool,
    },
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
        self.create_with_nnue_dir(None)
    }

    fn create_with_nnue_dir(&self, nnue_dir: Option<String>) -> Result<i64, BridgeError> {
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed) + 1;
        let mut config = load_default_config();
        SearchDifficulty::default().apply_to_config(&mut config);
        config.runtime.threads = default_interactive_threads();
        if let Some(dir) = nnue_dir.filter(|s| !s.trim().is_empty()) {
            config.runtime.nnue_dir = Some(dir);
            config.runtime.use_nnue = true;
        }
        let controller = Arc::new(Mutex::new(GameController::new(config).with_nnue_history()));
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
        let (request, language) = parse_request(request_json)?;
        let controller = self.controller(handle)?;
        dispatch_request(controller, request, language)
    }
}

fn registry() -> &'static BridgeRegistry {
    REGISTRY.get_or_init(BridgeRegistry::default)
}

fn parse_request(request_json: &str) -> Result<(BridgeRequest, UiLanguage), BridgeError> {
    let mut value: Value = serde_json::from_str(request_json)
        .map_err(|err| BridgeError::invalid_request(format!("Invalid request JSON: {err}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| BridgeError::invalid_request("Request JSON must be an object."))?;
    let language = match object.remove("lang") {
        Some(tag) => UiLanguage::from_tag(tag.as_str().ok_or_else(|| {
            BridgeError::invalid_request("Request field \"lang\" must be a string.")
        })?),
        None => UiLanguage::Chinese,
    };
    let operation = object
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::invalid_request("Request field \"op\" must be a string."))?;
    let allowed_fields: &[&str] = match operation {
        "state" | "undo" | "engine_move" => &["op"],
        "new_game" => &["op", "human_side", "rule", "mode", "clock_mode"],
        "play" => &["op", "x", "y"],
        "set_difficulty" => &["op", "difficulty"],
        "set_time_limit" => &["op", "time_limit_ms"],
        "set_threads" => &["op", "threads"],
        "set_multithread" => &["op", "enabled"],
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
        .map(|request| (request, language))
        .map_err(|err| BridgeError::invalid_request(format!("Invalid request fields: {err}")))
}

fn dispatch_request(
    controller: SharedController,
    request: BridgeRequest,
    language: UiLanguage,
) -> Result<Value, BridgeError> {
    match request {
        BridgeRequest::State => snapshot_response(&controller, language),
        BridgeRequest::NewGame {
            human_side,
            rule,
            mode,
            clock_mode,
        } => {
            let side = parse_side(&human_side)?;
            let rule = parse_rule(&rule)?;
            let mode = parse_mode(mode.as_deref())?;
            let clock_mode = parse_clock_mode(clock_mode.as_deref())?;
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .new_game_with_clock(side, rule, mode, clock_mode);
            snapshot_response(&controller, language)
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
            snapshot_response(&controller, language)
        }
        BridgeRequest::Undo => {
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .undo_turn();
            snapshot_response(&controller, language)
        }
        BridgeRequest::SetDifficulty { difficulty } => {
            let difficulty = parse_difficulty(&difficulty)?;
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_difficulty(difficulty);
            snapshot_response(&controller, language)
        }
        BridgeRequest::SetTimeLimit { time_limit_ms } => {
            if !SUPPORTED_TIME_LIMITS_MS.contains(&time_limit_ms) {
                return Err(BridgeError::invalid_request(
                    "time_limit_ms must be 500, 1000, 2500, or 5000.",
                ));
            }
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_time_limit_ms(time_limit_ms);
            snapshot_response(&controller, language)
        }
        BridgeRequest::SetThreads { threads } => {
            if threads == 0 || threads > MAX_SEARCH_THREADS {
                return Err(BridgeError::invalid_request(format!(
                    "threads must be between 1 and {MAX_SEARCH_THREADS}."
                )));
            }
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_threads(threads);
            snapshot_response(&controller, language)
        }
        BridgeRequest::SetMultithread { enabled } => {
            let threads = if enabled {
                default_interactive_threads()
            } else {
                1
            };
            controller
                .lock()
                .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
                .set_threads(threads);
            snapshot_response(&controller, language)
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
            snapshot_response(&controller, language)
        }
    }
}

fn snapshot_response(
    controller: &SharedController,
    language: UiLanguage,
) -> Result<Value, BridgeError> {
    let snapshot = controller
        .lock()
        .map_err(|_| BridgeError::internal("Game controller is unavailable."))?
        .snapshot_in(language);
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

fn parse_mode(value: Option<&str>) -> Result<GameMode, BridgeError> {
    match value {
        None | Some("vs_engine") => Ok(GameMode::VsEngine),
        Some("two_player") => Ok(GameMode::TwoPlayer),
        Some(_) => Err(BridgeError::invalid_request(
            "mode must be \"vs_engine\" or \"two_player\".",
        )),
    }
}

fn parse_clock_mode(value: Option<&str>) -> Result<SearchClockMode, BridgeError> {
    match value {
        None | Some("timed") | Some("pro") => Ok(SearchClockMode::Timed),
        Some("untimed") | Some("casual") | Some("fixed") => Ok(SearchClockMode::Untimed),
        Some(_) => Err(BridgeError::invalid_request(
            "clock_mode must be \"untimed\" or \"timed\".",
        )),
    }
}

fn parse_rule(value: &str) -> Result<RuleSet, BridgeError> {
    value
        .parse()
        .map_err(|_| BridgeError::invalid_request("rule must be \"freestyle\" or \"renju\"."))
}

fn parse_difficulty(value: &str) -> Result<SearchDifficulty, BridgeError> {
    value.parse().map_err(|_| {
        BridgeError::invalid_request(
            "difficulty must be \"beginner\", \"normal\", \"senior\", or \"master\".",
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
pub extern "system" fn Java_io_github_chubbykuu_rustgomoku_NativeBridge_nativeCreateWithNnueDir(
    mut env: JNIEnv,
    _class: JClass,
    nnue_dir: JString,
) -> jlong {
    catch_unwind(AssertUnwindSafe(|| {
        let dir = if nnue_dir.is_null() {
            None
        } else {
            env.get_string(&nnue_dir)
                .ok()
                .map(|s| s.to_string_lossy().into_owned())
        };
        registry().create_with_nnue_dir(dir).unwrap_or(0)
    }))
    .unwrap_or(0)
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
        assert_eq!(initial["state"]["params"]["clock_mode"], "timed");
        assert_eq!(initial["state"]["params"]["time_limit_ms"], 1000);
        assert_eq!(initial["state"]["params"]["difficulty"], "normal");
        assert_eq!(initial["state"]["params"]["use_nnue"], false);

        let casual = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"black","rule":"renju","clock_mode":"untimed"}"#,
        );
        assert_eq!(casual["state"]["params"]["rule"], "renju");
        assert_eq!(casual["state"]["params"]["clock_mode"], "untimed");
        assert_eq!(casual["state"]["params"]["difficulty"], "normal");
        assert_eq!(casual["state"]["params"]["depth"], 4);
        assert_eq!(casual["state"]["params"]["width"], 20);
        assert_eq!(casual["state"]["params"]["compute_vcf"], false);
        assert_eq!(casual["state"]["params"]["compute_vct"], false);
        assert_eq!(casual["state"]["human_side"], BLACK);

        let played = response(&registry, handle, r#"{"op":"play","x":7,"y":7}"#);
        assert_eq!(played["state"]["move_count"], 1);
        assert_eq!(played["state"]["side_to_move"], WHITE);

        let undone = response(&registry, handle, r#"{"op":"undo"}"#);
        assert_eq!(undone["state"]["move_count"], 0);

        let senior = response(
            &registry,
            handle,
            r#"{"op":"set_difficulty","difficulty":"senior"}"#,
        );
        assert_eq!(senior["state"]["params"]["difficulty"], "senior");
        assert_eq!(senior["state"]["params"]["depth"], 6);
        assert_eq!(senior["state"]["params"]["width"], 30);
        assert_eq!(senior["state"]["params"]["compute_vcf"], true);
        assert_eq!(senior["state"]["params"]["compute_vct"], false);

        let master = response(
            &registry,
            handle,
            r#"{"op":"set_difficulty","difficulty":"master"}"#,
        );
        assert_eq!(master["state"]["params"]["difficulty"], "master");
        assert_eq!(master["state"]["params"]["depth"], 8);
        assert_eq!(master["state"]["params"]["width"], 40);
        assert_eq!(master["state"]["params"]["compute_vcf"], true);
        assert_eq!(master["state"]["params"]["compute_vct"], true);
        assert_eq!(master["state"]["params"]["root_vct_depth"], 6);

        let threads = response(&registry, handle, r#"{"op":"set_threads","threads":1}"#);
        assert_eq!(threads["ok"], true);
        assert_eq!(threads["state"]["params"]["threads"], 1);

        let mt_on = response(
            &registry,
            handle,
            r#"{"op":"set_multithread","enabled":true}"#,
        );
        assert_eq!(mt_on["ok"], true);
        assert_eq!(
            mt_on["state"]["params"]["threads"],
            default_interactive_threads()
        );

        let mt_off = response(
            &registry,
            handle,
            r#"{"op":"set_multithread","enabled":false}"#,
        );
        assert_eq!(mt_off["ok"], true);
        assert_eq!(mt_off["state"]["params"]["threads"], 1);

        let timed = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"black","rule":"renju","mode":"vs_engine","clock_mode":"timed"}"#,
        );
        assert_eq!(timed["ok"], true);
        assert_eq!(timed["state"]["params"]["clock_mode"], "timed");
        assert_eq!(timed["state"]["params"]["time_limit_ms"], 1000);

        let time_updated = response(
            &registry,
            handle,
            r#"{"op":"set_time_limit","time_limit_ms":2500}"#,
        );
        assert_eq!(time_updated["ok"], true);
        assert_eq!(time_updated["state"]["params"]["time_limit_ms"], 2500);
    }

    #[test]
    fn two_player_mode_alternates_without_engine() {
        let registry = BridgeRegistry::default();
        let handle = registry.create().unwrap();

        let new_game = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"black","rule":"renju","mode":"two_player"}"#,
        );
        assert_eq!(new_game["state"]["params"]["mode"], "two_player");

        // Black then white are both placed by humans; the engine cannot move.
        let black = response(&registry, handle, r#"{"op":"play","x":7,"y":7}"#);
        assert_eq!(black["state"]["side_to_move"], WHITE);
        assert_eq!(black["state"]["can_play"], true);

        let engine = response(&registry, handle, r#"{"op":"engine_move"}"#);
        assert_eq!(engine["ok"], false);

        let white = response(&registry, handle, r#"{"op":"play","x":8,"y":8}"#);
        assert_eq!(white["state"]["move_count"], 2);
        assert_eq!(white["state"]["side_to_move"], BLACK);

        // A single undo hands the turn back to white.
        let undone = response(&registry, handle, r#"{"op":"undo"}"#);
        assert_eq!(undone["state"]["move_count"], 1);
        assert_eq!(undone["state"]["side_to_move"], WHITE);
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
            (
                r#"{"op":"new_game","human_side":"black","rule":"renju","mode":"solo"}"#,
                "invalid_request",
            ),
            (r#"{"op":"play","x":15,"y":0}"#, "invalid_request"),
            (
                r#"{"op":"set_difficulty","difficulty":"impossible"}"#,
                "invalid_request",
            ),
            (r#"{"op":"set_threads","threads":0}"#, "invalid_request"),
            (r#"{"op":"set_threads","threads":999}"#, "invalid_request"),
            (
                r#"{"op":"set_time_limit","time_limit_ms":750}"#,
                "invalid_request",
            ),
            (
                r#"{"op":"new_game","human_side":"black","rule":"renju","clock_mode":"blitz"}"#,
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
            dispatch_request(
                request_controller,
                BridgeRequest::State,
                UiLanguage::Chinese,
            )
            .unwrap()
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
            r#"{"op":"new_game","human_side":"white","rule":"renju","clock_mode":"untimed"}"#,
        );

        let moved = response(&registry, handle, r#"{"op":"engine_move"}"#);
        assert_eq!(moved["ok"], true);
        assert_eq!(moved["state"]["move_count"], 1);
        assert_eq!(moved["state"]["moves"][0]["x"], BOARD_SIZE / 2);
        assert_eq!(moved["state"]["moves"][0]["y"], BOARD_SIZE / 2);
    }

    #[test]
    fn create_with_nnue_dir_enables_nnue_in_pro_mode_only() {
        let dir = std::path::Path::new("../../opponent/rapfi-networks/mix9svq");
        if !rust_gomoku::nnue::mix9svq_files_exist_for_rule(dir, RuleSet::Renju) {
            return;
        }
        let registry = BridgeRegistry::default();
        let handle = registry
            .create_with_nnue_dir(Some(dir.to_string_lossy().into_owned()))
            .unwrap();

        let pro = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"white","rule":"renju","clock_mode":"timed"}"#,
        );
        assert_eq!(pro["ok"], true);
        assert_eq!(pro["state"]["params"]["nnue_available"], true);
        assert_eq!(pro["state"]["params"]["use_nnue"], true);

        let _ = response(
            &registry,
            handle,
            r#"{"op":"set_time_limit","time_limit_ms":500}"#,
        );
        let moved = response(&registry, handle, r#"{"op":"engine_move"}"#);
        assert_eq!(moved["ok"], true);
        assert_eq!(moved["state"]["last_result"]["used_nnue"], true);
        let history = moved["state"]["nnue_history"]
            .as_array()
            .expect("nnue_history should be recorded on Android Pro mode");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["ply"], 1);
        let sum = history[0]["black_win"].as_f64().unwrap()
            + history[0]["draw"].as_f64().unwrap()
            + history[0]["white_win"].as_f64().unwrap();
        assert!(
            (sum - 1.0).abs() < 1e-4,
            "probabilities should sum to 1, got {sum}"
        );

        let casual = response(
            &registry,
            handle,
            r#"{"op":"new_game","human_side":"black","rule":"renju","clock_mode":"untimed"}"#,
        );
        assert_eq!(casual["ok"], true);
        assert_eq!(casual["state"]["params"]["nnue_available"], true);
        assert_eq!(casual["state"]["params"]["use_nnue"], false);
    }
}
