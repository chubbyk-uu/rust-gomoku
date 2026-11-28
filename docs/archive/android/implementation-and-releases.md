# Android implementation phases and early releases

> Snapshot from the former [android-app-design.md](../../android-app-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
- difficulty changes while idle
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
- Invalid JSON, operations, fields, coordinates, sides, rules, difficulty values, and
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
3. Implement new game, play, undo, and difficulty requests.
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
4. Verify undo, restart, difficulty changes, and screen
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
release artifacts are attached to GitHub Releases. The latest published Android
package is `v0.1.20`.

1. Produce `app-debug.apk` for initial manual installation: complete.
2. After the real-device gate, create a release signing key outside the
   repository: complete on the local machine.
3. Configure release signing through local/CI secrets: complete for local
   builds.
4. Produce signed release APK and AAB: complete.
5. Attach the APK to a GitHub Release: complete through `v0.1.20`.
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
`versionName = "0.1.20"` and `versionCode = 20`.

GitHub Release:

- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.5>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.6> (Windows-only)
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.7>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.8>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.9>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.10>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.11>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.12> (Windows-only)
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.13>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.14>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.15>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.16>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.17>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.18>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.19>
- <https://github.com/chubbyk-uu/rust-gomoku/releases/tag/v0.1.20>

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
colors under both rules, forbidden marks and rejection, undo, restart, difficulty changes,
rotation and backgrounding during search, responsive framing, sustained
temperature, and representative search duration.
