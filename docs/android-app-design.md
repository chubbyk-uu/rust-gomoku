# Android App Design

## Status

This document defines the implementation plan for an Android version of
`rust_gomoku`. The Android application has not been implemented yet.

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

The existing Rust library has already completed an optimized ARM64 Android
cross-build. This proves the toolchain baseline only; it does not mean that a
JNI library or APK exists.

Implementation progress on `feature/android-app`:

- Phase 1 baseline is complete.
- Phase 2 shared controller extraction is complete.
- The desktop HTTP GUI now uses `GameController`.
- Controller tests cover forbidden input, first-move search, undo, profile
  switching, invalid sides, and stale search completion.
- Full Rust tests, Linux release builds, ARM64 Android cross-build, all 11 root
  diff cases, and a desktop HTTP workflow smoke passed after the refactor.
- No Gradle project, JNI library, Kotlin code, mobile assets, or APK exists yet.

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
│       ├── java/...             Activity, ViewModel, WebView bridge
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
engine_move
```

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

Status: next.

1. Add Gradle wrapper and application module.
2. Add a minimal Activity and local WebView page.
3. Configure API levels, ABI filtering, NDK version, and packaging.
4. Add Gradle task dependencies that build/copy the Rust library.
5. Produce an APK with a placeholder bridge.

Gate:

- `./gradlew assembleDebug` succeeds in WSL.
- APK contains only `arm64-v8a` native code.
- Manifest has no network permission.

### Phase 4: Rust JNI Bridge

1. Add the `android/rust_bridge` `cdylib`.
2. Implement handle creation, request dispatch, and destruction.
3. Connect the bridge to the shared controller.
4. Add panic containment and structured errors.
5. Test JSON commands on Linux where possible and cross-build the JNI library.

Gate:

- JNI library is an AArch64 Android ELF shared object.
- Invalid requests do not panic.
- Controller tests and desktop GUI tests still pass.

### Phase 5: Kotlin Bridge And Search Thread

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

APK inspection:

- correct application id and version
- only expected ABI
- JNI library present
- no unexpected permissions
- local assets present
- debug APK signature valid

### Phase 8: Real-Device Gate

Manual checklist:

1. Play both colors under freestyle.
2. Play both colors under Renju.
3. Verify overline, double-four, and double-three marks and rejection.
4. Verify undo, restart, profile switch, and screen rotation.
5. Put the application in the background during search and return.
6. Run for 20-30 minutes and observe heat, battery use, memory, and crashes.
7. Compare selected fixed positions with desktop best moves.
8. Record search duration on at least one representative phone.

Do not reduce default depth or disable VCF/VCT based on assumptions. If the
phone result is too slow or hot, introduce an explicit mobile strength policy
and validate its playing strength separately.

### Phase 9: Distribution

1. Produce `app-debug.apk` for initial manual installation.
2. After the real-device gate, create a release signing key outside the
   repository.
3. Configure release signing through local/CI secrets.
4. Produce signed release APK and AAB.
5. Attach the APK to a GitHub Release.
6. Consider Play publication only after signing, privacy, screenshots, version
   upgrades, and device compatibility are stable.

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

Implement Phase 3 on `feature/android-app`: add the Gradle wrapper and minimal
Android application, package only `arm64-v8a`, and prove that
`./gradlew assembleDebug` produces an APK containing an optimized Rust
placeholder library without requesting network access.
