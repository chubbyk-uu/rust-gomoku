# Android guide

The Android ARM64 app uses a local WebView for the board and controls, Kotlin for the lifecycle and request queue, and the same Rust game controller as the desktop application. See the [engine guide](engine.md#interactive-presets) for Pro and Casual modes.

## Build

The checked-in build uses JDK 17, Gradle 9.4.1 through the wrapper, Android SDK 36 (minimum runtime API 26), Build Tools 36.0.0, and NDK 29.0.14206865. Install Node.js for the mobile UI tests, Rust's `aarch64-linux-android` target, and `cargo-ndk`. The Gradle build compiles an optimized ARM64 Rust library for either build variant.

```bash
cd android
./gradlew test lint assembleDebug
```

Gradle stages the [Mix9svq files](engine.md#nnue-weights) from `RUST_GOMOKU_NNUE_DIR` or `opponent/rapfi-networks/mix9svq`. `GameViewModel` extracts the packaged assets into the app's files directory. A build without those assets can use classical evaluation.

### Signed builds

Set up the properties file outside the repository. The default is `~/.android/rust-gomoku-release.properties`. `RUST_GOMOKU_ANDROID_SIGNING_PROPERTIES` can point to another file. It must define `storeFile`, `storePassword`, `keyAlias`, and `keyPassword`.

```bash
cd android
./gradlew test lint assembleRelease bundleRelease
```

Outputs are `android/app/build/outputs/apk/release/app-release.apk` and `android/app/build/outputs/bundle/release/app-release.aab`, relative to the repository root. Download distributed builds from [GitHub Releases](https://github.com/chubbyk-uu/rust-gomoku/releases/latest). See [`android/app/build.gradle.kts`](../android/app/build.gradle.kts) for packaging and version fields.

## Architecture and validation

- [`src/app.rs`](../src/app.rs) owns game state, options, engine requests, and stale-result protection.
- [`android/rust_bridge/src/lib.rs`](../android/rust_bridge/src/lib.rs) exposes `nativeCreate`, `nativeCreateWithNnueDir`, `nativeRequest`, and `nativeDestroy`. Requests use a strict JSON schema. A successful request returns `{"ok":true,"state":...}`. Bridge errors use a structured `error` object.
- Kotlin keeps the native handle in the ViewModel across configuration changes and serializes requests on a background thread. The WebView loads bundled content through `WebViewAssetLoader` and receives state on the UI thread.
- The WebView restricts navigation to the bundled asset origin and disables file and content access. The app manifest has no Internet permission.
- The mobile UI supports both orientations, tap-to-preview and tap-to-confirm, forbidden-point indicators, and Pro win-rate estimates. `testMobileUi` covers interface state. For mobile UI changes, check rotation, busy controls, and undo on a device.

The initial delivery plan and device checks are in the [Android archive](archive/README.md#android).
