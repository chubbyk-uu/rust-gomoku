package io.github.chubbykuu.rustgomoku

import android.app.Application
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.AndroidViewModel
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.Executors

/**
 * Owns the native game handle across configuration changes (e.g. rotation) and
 * serializes every native request onto a single background thread so asset
 * extraction and engine search never run on the Android main thread.
 *
 * Staleness and rule legality are enforced inside the shared Rust controller
 * through its generation counter; this class only guarantees that callbacks are
 * delivered on the main thread and are dropped once the handle is released.
 */
class GameViewModel(application: Application) : AndroidViewModel(application) {
    // Lazily created on the single background executor after ensuring bundled
    // NNUE weight files are extracted into app-private storage.
    private var handle: Long = 0L
    private var handleInitialized = false

    // Single thread => all controller operations (init, state, play, undo, the
    // blocking engine_move search) run strictly one at a time, off the UI thread.
    private val executor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile
    private var destroyed = false

    private fun ensureHandle(): Long {
        if (!handleInitialized) {
            handleInitialized = true
            val nnueDir = extractBundledNnueWeights()
            handle = if (nnueDir != null) {
                NativeBridge.nativeCreateWithNnueDir(nnueDir)
            } else {
                NativeBridge.nativeCreate()
            }
        }
        return handle
    }

    /**
     * Copy bundled Mix9svq `.bin.lz4` weights from `assets/nnue/` to
     * `filesDir/nnue/` once per install/upgrade so Rust's path-based NNUE
     * loader can read them directly.
     */
    private fun extractBundledNnueWeights(): String? {
        return try {
            val app = getApplication<Application>()
            val assets = app.assets
            val assetEntries = assets.list(NNUE_ASSET_DIR)
                ?.filter { it.endsWith(".bin.lz4") }
                .orEmpty()
            if (assetEntries.isEmpty()) {
                return null
            }
            val targetDir = File(app.filesDir, NNUE_ASSET_DIR)
            if (!targetDir.exists() && !targetDir.mkdirs()) {
                return null
            }
            val buffer = ByteArray(COPY_BUFFER_BYTES)
            for (name in assetEntries) {
                val assetPath = "$NNUE_ASSET_DIR/$name"
                val destFile = File(targetDir, name)
                val expectedLength = try {
                    assets.openFd(assetPath).use { it.length }
                } catch (_: Throwable) {
                    -1L
                }
                val alreadyUpToDate = destFile.isFile && (
                    if (expectedLength > 0L) destFile.length() == expectedLength
                    else destFile.length() > 0L
                )
                if (alreadyUpToDate) {
                    continue
                }
                val tmpFile = File(targetDir, "$name.tmp")
                assets.open(assetPath).use { input ->
                    FileOutputStream(tmpFile).use { output ->
                        while (true) {
                            val read = input.read(buffer)
                            if (read <= 0) break
                            output.write(buffer, 0, read)
                        }
                        output.fd.sync()
                    }
                }
                if (destFile.exists()) {
                    destFile.delete()
                }
                if (!tmpFile.renameTo(destFile)) {
                    tmpFile.delete()
                    return null
                }
            }
            targetDir.absolutePath
        } catch (_: Throwable) {
            null
        }
    }

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
        try {
            executor.execute {
                val currentHandle = try {
                    ensureHandle()
                } catch (_: Throwable) {
                    0L
                }
                val response = if (currentHandle == 0L) {
                    HANDLE_ERROR
                } else {
                    try {
                        NativeBridge.nativeRequest(currentHandle, requestJson) ?: INTERNAL_ERROR
                    } catch (_: Throwable) {
                        INTERNAL_ERROR
                    }
                }
                mainHandler.post { if (!destroyed) onResult(response) }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            // Executor already shutting down during teardown; nothing to deliver.
        }
    }

    override fun onCleared() {
        destroyed = true
        // Queued behind any in-flight/pending requests on the single thread,
        // so prior work finishes against a still-valid handle first.
        executor.execute {
            if (handle != 0L) {
                NativeBridge.nativeDestroy(handle)
                handle = 0L
            }
        }
        executor.shutdown()
        super.onCleared()
    }

    private companion object {
        const val NNUE_ASSET_DIR = "nnue"
        const val COPY_BUFFER_BYTES = 64 * 1024
        const val INTERNAL_ERROR =
            """{"ok":false,"error":{"code":"internal_error","message":"Native request failed."}}"""
        const val HANDLE_ERROR =
            """{"ok":false,"error":{"code":"invalid_handle","message":"Native game handle could not be created."}}"""
    }
}
