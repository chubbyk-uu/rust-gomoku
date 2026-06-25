package io.github.chubbykuu.rustgomoku

/**
 * Thin declaration of the Rust JNI contract implemented in
 * `android/rust_bridge/src/lib.rs`. The exported symbols are
 * `Java_io_github_chubbykuu_rustgomoku_NativeBridge_native*`, so this object's
 * fully qualified name must stay `io.github.chubbykuu.rustgomoku.NativeBridge`.
 *
 * The native surface is intentionally tiny: create a handle, exchange JSON
 * request/response strings, destroy the handle. All game, rule, and search
 * logic lives in the shared Rust controller; Kotlin never reimplements it.
 */
object NativeBridge {
    init {
        System.loadLibrary("rust_gomoku_android")
    }

    /** Allocate a native game controller and return its handle (0 on failure). */
    external fun nativeCreate(): Long

    /**
     * Run a JSON command against the handle and return the JSON response.
     * Returns `null` only if the native side failed to allocate the response
     * string; callers treat that as an internal error.
     */
    external fun nativeRequest(handle: Long, requestJson: String): String?

    /** Release the handle. Any in-flight request keeps its controller alive. */
    external fun nativeDestroy(handle: Long)
}
