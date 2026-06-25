package io.github.chubbykuu.rustgomoku

import android.os.Handler
import android.os.Looper
import androidx.lifecycle.ViewModel
import java.util.concurrent.Executors

/**
 * Owns the native game handle across configuration changes (e.g. rotation) and
 * serializes every native request onto a single background thread so engine
 * search never runs on the Android main thread.
 *
 * Staleness and rule legality are enforced inside the shared Rust controller
 * through its generation counter; this class only guarantees that callbacks are
 * delivered on the main thread and are dropped once the handle is released.
 */
class GameViewModel : ViewModel() {
    private val handle: Long = NativeBridge.nativeCreate()

    // Single thread => all controller operations (state, play, undo, the
    // blocking engine_move search) run strictly one at a time, off the UI thread.
    private val executor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile
    private var destroyed = false

    /**
     * Run a JSON command on the background executor and deliver the JSON
     * response on the main thread. Stale callbacks (after [onCleared]) are
     * dropped. If the handle could not be created the request short-circuits to
     * a structured error so the UI still receives a well-formed response.
     */
    fun request(requestJson: String, onResult: (String) -> Unit) {
        if (destroyed) {
            return
        }
        if (handle == 0L) {
            mainHandler.post { if (!destroyed) onResult(HANDLE_ERROR) }
            return
        }
        try {
            executor.execute {
                val response = try {
                    NativeBridge.nativeRequest(handle, requestJson) ?: INTERNAL_ERROR
                } catch (throwable: Throwable) {
                    INTERNAL_ERROR
                }
                mainHandler.post { if (!destroyed) onResult(response) }
            }
        } catch (rejected: java.util.concurrent.RejectedExecutionException) {
            // Executor already shutting down during teardown; nothing to deliver.
        }
    }

    override fun onCleared() {
        destroyed = true
        if (handle != 0L) {
            // Queued behind any in-flight/pending requests on the single thread,
            // so prior work finishes against a still-valid handle first.
            executor.execute { NativeBridge.nativeDestroy(handle) }
        }
        executor.shutdown()
        super.onCleared()
    }

    private companion object {
        const val INTERNAL_ERROR =
            """{"ok":false,"error":{"code":"internal_error","message":"Native request failed."}}"""
        const val HANDLE_ERROR =
            """{"ok":false,"error":{"code":"invalid_handle","message":"Native game handle could not be created."}}"""
    }
}
