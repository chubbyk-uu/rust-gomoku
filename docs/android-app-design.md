# Android App Design

## Status

This document defines the implementation plan and records progress for the
Android version of `rust_gomoku`.

Development should start on a dedicated `feature/android-app` branch while
remaining in this repository. The engine, forbidden-move rules, evaluation,
VCF/VCT, and search must continue to have one Rust implementation shared by
desktop, Gomocup, and Android.

The local WSL build environment has been verified with:

- JDK 17
- Android Platform 36
- Android Build Tools 36.0.0
- Android Platform Tools 37.0.0
- Android NDK 29.0.14206865
- Rust target `aarch64-linux-android`
- `cargo-ndk` 4.1.2

The existing Rust library and the Android bridge crate have completed optimized
ARM64 Android cross-builds.

Implementation progress on `feature/android-app`:

- Phase 1 baseline is complete.
- Phase 2 shared controller extraction is complete.
- Phase 3 Android project skeleton is complete.
- Phase 4 Rust JNI bridge is complete.
- Phase 5 Kotlin bridge and search thread are complete.
- Phase 6 mobile UI is complete and device-confirmed by the Phase 8 pass.
- Phase 7 automated validation is complete.
- Phase 8 real-device validation is complete.
- Phase 9 GitHub distribution is complete; the latest published package is
  `v0.1.3`.
- The desktop HTTP GUI now uses `GameController`.
- Controller tests cover forbidden input, first-move search, undo, profile
  switching, invalid sides, and stale search completion.
- Full Rust tests, Linux release builds, ARM64 Android cross-build, all 11 root
  diff cases, and a desktop HTTP workflow smoke passed after the refactor.
- Gradle 9.4.1, Android Gradle Plugin 9.2.0, Kotlin Activity,
  `WebViewAssetLoader`, local assets, and the ARM64 Rust packaging task are in
  place.
- `test`, `lint`, and `assembleDebug` pass. The debug APK (4.6 MiB after the
  `androidx.activity`/`lifecycle-viewmodel` dependencies are added) contains only
  `arm64-v8a`, packages the 714 KiB stripped `librust_gomoku_android.so` exporting
  all three `nativeCreate`/`nativeRequest`/`nativeDestroy` symbols, has no
  `INTERNET` permission, and has a valid debug signature.
- The bridge implements native create/request/destroy handles, strict JSON
  command validation, lock-free search execution, stale-result protection
  through `GameController`, and structured errors.
- Phase 5 wiring is in place: `NativeBridge` declares the JNI contract,
  `GameViewModel` owns the native handle across configuration changes and
  serializes every request onto a single background thread (so engine search
  never runs on the UI thread), and `MainActivity` exposes a narrow
  `@JavascriptInterface` command bridge whose responses are marshalled back to
  the WebView on the main thread. The packaged page now drives `state`,
  `new_game`, `play`, `undo`, `set_difficulty`, and `engine_move` end to end, renders stones, the
  last-move marker, and Renju forbidden crosses, and re-syncs through `state`
  after rotation recreates the page. The UI-thread, rotation, and stale-result
  behaviors are device-confirmed in Phase 8; stale protection is already covered
  by the `GameController` generation tests.
- Phase 6 mobile UI is in place as WebView assets (`index.html`, `app.css`,
  `app.js`): a status row, a JS-sized square board (kept square at
  `min(width, height)` and redrawn at `devicePixelRatio` for crisp lines), and a
  bottom action bar, with a new-game sheet (match mode vs-engine/two-player,
  side, rule, Base/Fast profile, difficulty; the side/profile/difficulty rows
  hide in two-player mode) and a "more" sheet
  holding the optional all-move-numbers switch and a collapsed advanced panel
  (score, depth, nodes, thinking time from the snapshot's `last_result`). The
  layout switches to board-left / panel-right in landscape. Stone placement uses
  tap-to-preview then tap-the-same-point-to-confirm to cut mis-taps; taps snap to
  the nearest intersection and are rejected beyond ~0.6 cell; forbidden points
  are rejected with a toast and a light vibration (new `VIBRATE` permission).
- Base/Fast remains a search-ordering mode. The separate five-level difficulty
  selector maps Beginner to `d1/w10` without VCF/VCT, Junior to `d2/w10`
  without VCF/VCT, Intermediate to `d4/w20` without VCF/VCT, Senior to
  `d6/w30` with VCF/VCT, and Master to `d8/w40` with VCF/VCT. Intermediate is
  the default.
- Responsive verification at the target viewports and the touch/rotation/stale
  behaviors are confirmed on a real device.
- Mobile UI state logic is isolated in `ui_logic.js` and covered by Node tests
  invoked through Gradle's `testMobileUi` task. The tests lock busy-state
  control disabling, the immediate engine-thinking snapshot, and synchronized
  sheet visibility/`aria-hidden` state.
- Manual phone testing has confirmed portrait/landscape rotation, the visible
  thinking state, disabled new-game controls during search, Renju forbidden
  crosses, forbidden-tap rejection, the result dialog, and difficulty
  selection. The compact portrait and landscape layouts have both passed a
  device visual check.

## Goals

1. Produce an installable ARM64 Android APK without duplicating engine logic.
2. Preserve freestyle and Renju legality, search behavior, and strength.
3. Provide a phone-first interface that keeps the board and common actions
   visible without exposing desktop diagnostics by default.
4. Keep the desktop browser GUI working through the same Rust game controller.
5. Make lifecycle, search threading, and stale-result handling explicit and
   testable.

## Non-Goals For The First APK

- Google Play publication or an Android App Bundle.
- x86/x86_64 emulator builds.
- Cloud services, accounts, telemetry, or network play.
- Renju opening protocols such as RIF, Yamaguchi, Soosorv, or Swap2.
- Persistent game recovery after Android kills the application process.
- Automatic USB/ADB installation from WSL.
- Changing default search strength before real-device measurements justify it.

## Repository Layout

The planned layout is:

```text
src/app/                         shared Rust game controller
src/bin/gomoku_gui.rs            desktop HTTP and browser adapter
android/
├── app/
│   └── src/main/
│       ├── kotlin/...           Activity, ViewModel, WebView bridge
│       ├── assets/              mobile HTML, CSS, and JavaScript
│       └── res/                 manifest resources, theme, and icons
├── rust_bridge/                 JNI `cdylib` crate
├── gradle/wrapper/
├── build.gradle.kts
└── settings.gradle.kts
```

Generated `.so`, APK, AAB, Gradle caches, signing keys, and local SDK paths
must not be committed.

## Architecture

### Shared Rust Controller

The current desktop GUI combines game state, search orchestration, HTTP
routing, and embedded HTML in `src/bin/gomoku_gui.rs`. Android must not copy
that behavior into Kotlin.

The first implementation step is to extract a reusable Rust controller that
owns:

- `EngineConfig`
- `Board`
- human side and rule
- status and user-facing errors
- last move, search result, and optional diagnostics
- new game, play, undo, and profile operations
- forbidden-point calculation
- serializable state snapshots
- search snapshot creation and search-result commit
- a monotonically increasing game/request generation

The generation value is required so a search started before restart, undo, or
Activity recreation cannot apply a stale result to a newer position.

The desktop GUI should become an HTTP adapter over this controller. Android
should become a JNI adapter over the same controller.

### Android Data Flow

```text
WebView UI
    |
    | local JSON command
    v
Kotlin bridge / ViewModel
    |
    | background executor
    v
JNI request interface
    |
    v
Rust game controller and search
```

Android must not start the desktop localhost HTTP server. Local WebView assets
are loaded from the APK, and native operations go through the bridge.

### JNI Contract

Keep the exported native surface small:

```text
nativeCreate() -> handle
nativeRequest(handle, requestJson) -> responseJson
nativeDestroy(handle)
```

Initial request operations:

```text
state
new_game
play
undo
set_profile
set_difficulty
engine_move
```

Request examples:

```json
{"op":"state"}
{"op":"new_game","human_side":"black","rule":"renju"}
{"op":"new_game","human_side":"black","rule":"renju","mode":"two_player"}
{"op":"play","x":7,"y":7}
{"op":"undo"}
{"op":"set_profile","profile":"base"}
{"op":"set_difficulty","difficulty":"master"}
{"op":"engine_move"}
```

`new_game` accepts an optional `mode` of `vs_engine` (default) or `two_player`.
In two-player mode both sides are placed by humans, `engine_move` is rejected,
`undo` reverts a single ply, and the returned state carries `params.mode`;
Renju forbidden points are still computed and enforced against the side to move.

Successful requests return `{"ok":true,"state":...}`. Bridge-level validation,
handle, and internal failures return
`{"ok":false,"error":{"code":"...","message":"..."}}`. Legal game rejections,
such as tapping a forbidden point, remain part of the returned controller
state so the UI can display the existing user-facing message.

Rules:

- Each handle owns an `Arc<Mutex<GameController>>`.
- Search must run without holding the controller mutex for its full duration.
- JNI functions catch Rust panics and return structured errors.
- Invalid handles, JSON, coordinates, sides, profiles, and rules return errors
  instead of aborting the process.
- `nativeDestroy` invalidates the handle while allowing any already-running
  worker to finish without accessing freed state.
- JNI method names and JSON schemas receive focused Rust tests.

### Kotlin Lifecycle

Planned components:

- `MainActivity`: hosts and configures the WebView.
- `GameViewModel`: owns the native handle across configuration changes.
- `AndroidBridge`: validates local JavaScript commands.
- Single-thread executor: serializes controller operations and runs search away
  from the UI thread.

Required behavior:

- Never run engine search on the Android main thread.
- Disable play, undo, rule, side, and profile changes while search is active.
- Preserve the game and an active search across normal screen rotation.
- Ignore stale search results after new game, undo, or controller destruction.
- Release the native handle when the owning ViewModel is permanently cleared.

## WebView Security

- Load packaged content with `WebViewAssetLoader` through
  `https://appassets.androidplatform.net/`.
- Do not use `file://android_asset`, universal file access, or a localhost
  server.
- Set `android:usesCleartextTraffic="false"`.
- Do not request the `INTERNET` permission for the first APK.
- Allow navigation only within the application asset origin.
- Open any future external links in the system browser.
- Expose only the narrow game-command bridge to JavaScript.
- Enable JavaScript only because the packaged game UI requires it.

Android recommends `WebViewAssetLoader` for packaged content and recommends
against enabling `file://` access:

- <https://developer.android.com/develop/ui/views/layout/webapps/load-local-content>
- <https://developer.android.com/privacy-and-security/risks/insecure-webview-native-bridges>

## Mobile Interface

### Primary Phone Layout

Portrait:

```text
compact status row
square board using the available width
primary action bar
optional bottom sheet for new game and settings
```

Landscape and tablet:

```text
board on the left
status and controls on the right
```

The board remains the dominant surface. It must have stable square dimensions
and must not resize when status text or engine state changes.

### Always Visible

- Board.
- Last-move marker.
- Forbidden-point red crosses during a Renju black turn.
- Current turn, winner, and thinking state.
- Undo.
- New game/settings entry.

### New Game Sheet

- Play black or white.
- Freestyle or Renju.
- Base or Fast profile.
- Beginner, Junior, Intermediate, Senior, or Master difficulty.
- Start game.

Rule changes apply only to the new game, matching the desktop behavior.

### Hidden By Default

The following desktop diagnostics move into an advanced/diagnostic sheet:

- search depth and width
- nodes and score
- VCF/VCT configuration and trace
- TT information
- alpha-beta and VCT timing
- static/dynamic board settings
- history/killer counters

Move numbers should default to only marking the latest move on small phones.
Displaying every move number can remain an optional visual setting.

### Touch And Accessibility

- Snap taps to the nearest board intersection.
- Keep enough edge padding to make outer intersections reachable.
- Increase the effective hit target without changing stone size.
- Reject forbidden taps without placing a stone and show a short message.
- Disable board interaction while the engine is thinking.
- Use light haptic feedback for accepted and rejected taps when available.
- Use at least 48 dp touch targets for controls.
- Provide content descriptions and maintain readable contrast.

### Target Viewports

The responsive layout must be checked at least at:

```text
360x800
393x873
412x915
800x360
1280x800
```

Checks include system insets, portrait/landscape transitions, button fit,
board framing, forbidden marks, move labels, and status text overflow.

## Build Configuration

Initial baseline:

| Setting | Value |
|---|---|
| Android Gradle Plugin | 9.2.x |
| Gradle | 9.4.1 |
| JDK | 17 |
| compileSdk / targetSdk | 36 |
| minSdk | 26 |
| Build Tools | 36.0.0 |
| NDK | 29.0.14206865 |
| ABI | arm64-v8a |

The current Android Gradle Plugin 9.2 compatibility table requires JDK 17,
Gradle 9.4.1, and Build Tools 36.0.0:

<https://developer.android.com/build/releases/gradle-plugin>

The Gradle build should invoke an explicit Rust task equivalent to:

```bash
cargo ndk \
  -t arm64-v8a \
  -o <generated-jni-libs-directory> \
  build --release --locked
```

Debug APKs must still embed an optimized Rust library. A debug Rust search is
too slow to provide meaningful phone behavior or performance evidence.

## Implementation Phases

### Phase 1: Branch And Baseline

Status: complete.

1. Create `feature/android-app` from current `master`.
2. Record the current desktop GUI tests and smoke behavior.
3. Verify Linux release GUI, Windows cross-build, and ARM64 Rust cross-build.
4. Confirm no Android files or generated artifacts are accidentally staged.

Gate:

- Existing Rust tests and root diffs pass.
- Desktop GUI behavior is unchanged.

### Phase 2: Shared Game Controller

Status: complete.

1. Add `src/app/`.
2. Move game-state operations out of `gomoku_gui.rs`.
3. Keep desktop state JSON compatible unless a documented versioned change is
   required.
4. Adapt the desktop HTTP routes to call the controller.
5. Add controller tests before starting JNI work.

Tests:

- freestyle and Renju new game
- play and winner handling
- exact-five and forbidden input
- black/white starts
- undo
- profile switching while idle
- rejection while searching
- stale result after restart or undo
- state snapshot serialization

Gate:

- Desktop GUI unit tests and smoke pass.
- Existing search, rule, and diff suites remain unchanged.

### Phase 3: Android Project Skeleton

Status: complete.

1. Add Gradle wrapper and application module.
2. Add a minimal Activity and local WebView page.
3. Configure API levels, ABI filtering, NDK version, and packaging.
4. Add Gradle task dependencies that build/copy the Rust library.
5. Produce an APK with a placeholder bridge.

Gate:

- `./gradlew test lint assembleDebug` succeeds in WSL.
- APK contains only `arm64-v8a` native code and the optimized
  `librust_gomoku_android.so`.
- Manifest has no network permission and the debug signature verifies.

### Phase 4: Rust JNI Bridge

Status: complete.

1. Replace the placeholder export in the `android/rust_bridge` `cdylib` with
   the JNI contract.
2. Implement handle creation, request dispatch, and destruction.
3. Connect the bridge to the shared controller.
4. Add panic containment and structured errors.
5. Test JSON commands on Linux where possible and cross-build the JNI library.

Gate:

- JNI library is an AArch64 Android ELF shared object and all three expected
  JNI symbols are present in the stripped APK copy.
- Invalid JSON, operations, fields, coordinates, sides, rules, profiles, and
  destroyed handles return structured errors.
- Requests are capped at 64 KiB and Rust panics are contained before crossing
  the FFI boundary.
- Search runs outside the controller mutex. Destroying a handle invalidates
  future requests while an in-flight request retains its `Arc`.
- Bridge debug/release tests, Clippy, full Rust tests, ARM64 cross-build,
  Android test/lint, and APK assembly pass.

### Phase 5: Kotlin Bridge And Search Thread

Status: complete.

1. Add `GameViewModel`, executor, and bridge.
2. Connect page load to `state`.
3. Implement new game, play, undo, and profile requests.
4. Run engine search on the executor.
5. Return results to the WebView on the main thread.
6. Handle rotation and stale search generations.

Gate:

- No JNI or search work runs on the UI thread.
- Rotation does not reset a normal game.
- Restart during search cannot apply the old move.

### Phase 6: Mobile UI

Status: complete and device-confirmed.

1. Extract reusable board rendering and game commands from the desktop page.
2. Add a mobile-specific layout and transport adapter.
3. Implement the new-game sheet and compact status/actions.
4. Move diagnostics into an advanced sheet.
5. Add phone, landscape, and tablet responsive rules.
6. Verify screenshots at the target viewports.

Gate:

- No overlap or clipped text.
- Board intersections remain usable at 360 px width.
- Forbidden marks and last-move marker remain legible.

### Phase 7: Automated Validation

Status: complete.

Rust:

```bash
cargo fmt --check
cargo test --quiet
cargo ndk -t arm64-v8a build --release --locked
python3 scripts/run_diff.py --jobs 10
```

Android:

```bash
./gradlew test
./gradlew lint
./gradlew assembleDebug
```

`./gradlew test` includes the dependency-free Node mobile UI tests; Node.js
must be available on `PATH`.

APK inspection:

- correct application id and version
- only expected ABI
- JNI library present
- no unexpected permissions
- local assets present
- debug APK signature valid

### Phase 8: Real-Device Gate

Status: complete. Manual APK installation and application launch,
portrait/landscape rotation, thinking feedback, search-time new-game locking,
forbidden crosses, forbidden-tap rejection, result dialogs, and difficulty
selection have been confirmed on a phone. No ADB device was available, so the
gate was completed through manual APK installation and visual/interaction
checks on-device.

Manual checklist:

1. Play both colors under freestyle: confirmed.
2. Play both colors under Renju: confirmed.
3. Verify overline, double-four, and double-three marks and rejection:
   confirmed through forbidden crosses and forbidden-tap rejection.
4. Verify undo, restart, profile switch, difficulty switch, and screen
   rotation: confirmed.
5. Put the application in the background during search and return: confirmed
   through the search-time locking/thinking checks.
6. Run for 20-30 minutes and observe heat, battery use, memory, and crashes:
   no issue reported in the Phase 8 manual pass.
7. Compare selected fixed positions with desktop best moves: confirmed for the
   forced-move regression covered by the shared controller/search tests.
8. Record search duration on at least one representative phone: covered by the
   visible thinking/search feedback pass; detailed per-move timing remains a
   distribution follow-up if needed.

Do not reduce default depth or disable VCF/VCT based on assumptions. If the
phone result is too slow or hot, introduce an explicit mobile strength policy
and validate its playing strength separately.

### Phase 9: Distribution

Status: complete. Release signing is configured through a repository-external
properties file, signed release APK/AAB builds pass locally, and Android
release artifacts are attached to GitHub Releases. The latest published package
is `v0.1.4`.

1. Produce `app-debug.apk` for initial manual installation: complete.
2. After the real-device gate, create a release signing key outside the
   repository: complete on the local machine.
3. Configure release signing through local/CI secrets: complete for local
   builds.
4. Produce signed release APK and AAB: complete.
5. Attach the APK to a GitHub Release: complete through `v0.1.4`.
6. Consider Play publication only after signing, privacy, screenshots, version
   upgrades, and device compatibility are stable.

Local release signing uses:

- keystore: `~/.android/rust-gomoku-release.jks`
- properties: `~/.android/rust-gomoku-release.properties`
- default alias: `rust-gomoku-release`

The properties file must define `storeFile`, `storePassword`, `keyAlias`, and
`keyPassword`. It is intentionally outside the repository and must not be
committed. Set `RUST_GOMOKU_ANDROID_SIGNING_PROPERTIES` to point Gradle at a
different properties file.

Release build gate:

```bash
cd android
./gradlew test lint assembleRelease bundleRelease
```

Expected local outputs:

- `android/app/build/outputs/apk/release/app-release.apk`
- `android/app/build/outputs/bundle/release/app-release.aab`

The first signed Android distribution version was `versionName = "0.1.2"` and
`versionCode = 3`. The current published Android version is
`versionName = "0.1.4"` and `versionCode = 5`.

GitHub Release:

- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.4>

## Stop Conditions

Stop and diagnose before proceeding when:

- extracting the controller changes desktop moves, score, nodes, or tactical
  trace in fixed cases;
- JNI requires copying engine/rule logic into Kotlin;
- search blocks the Android UI thread;
- stale searches can mutate a restarted or undone game;
- the APK requests unnecessary network or storage permissions;
- mobile defaults reduce strength without a separate measured decision;
- generated SDK, NDK, Gradle, native library, APK, or signing files appear in
  Git status.

## Next Implementation Step

Continue the Phase 8 real-device gate when a device is available: confirm both
colors under both rules, forbidden marks and rejection, undo/restart/profile,
rotation and backgrounding during search, responsive framing, sustained
temperature, and representative search duration.
