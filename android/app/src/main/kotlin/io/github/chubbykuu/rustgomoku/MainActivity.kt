package io.github.chubbykuu.rustgomoku

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import androidx.lifecycle.ViewModelProvider
import androidx.webkit.WebViewAssetLoader
import org.json.JSONObject

class MainActivity : ComponentActivity() {
    private lateinit var webView: WebView
    private lateinit var viewModel: GameViewModel
    private var bridge: WebAppBridge? = null

    companion object {
        private const val ASSET_HOST = "appassets.androidplatform.net"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Survives configuration changes, so the native handle and any in-flight
        // engine search outlive screen rotation.
        viewModel = ViewModelProvider(this)[GameViewModel::class.java]

        val assetLoader = WebViewAssetLoader.Builder()
            .addPathHandler(
                "/assets/",
                WebViewAssetLoader.AssetsPathHandler(this),
            )
            .build()

        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.domStorageEnabled = true // Stores the local UI language preference.
            // Allow the synthesized stone-placement sound to play; the page still
            // resumes its AudioContext from a user gesture.
            settings.mediaPlaybackRequiresUserGesture = false
            webViewClient = LocalContentWebViewClient(assetLoader)
        }

        bridge = WebAppBridge(viewModel, webView).also {
            // Only the packaged asset origin can reach this interface; JavaScript
            // is enabled solely for the bundled game UI.
            webView.addJavascriptInterface(it, "AndroidBridge")
        }

        val systemLanguage = if (resources.configuration.locales[0].language == "zh") "zh-CN" else "en"
        webView.loadUrl("https://$ASSET_HOST/assets/index.html?lang=$systemLanguage&version=${BuildConfig.VERSION_NAME}")
        setContentView(webView)
    }

    override fun onDestroy() {
        bridge?.release()
        bridge = null
        webView.destroy()
        super.onDestroy()
    }

    private class LocalContentWebViewClient(
        private val assetLoader: WebViewAssetLoader,
    ) : WebViewClient() {
        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest,
        ): WebResourceResponse? = assetLoader.shouldInterceptRequest(request.url)

        override fun shouldOverrideUrlLoading(
            view: WebView,
            request: WebResourceRequest,
        ): Boolean {
            val uri = request.url
            if (isApplicationAsset(uri)) return false
            if (request.isForMainFrame && request.hasGesture() && isProjectLink(uri)) {
                try {
                    view.context.startActivity(Intent(Intent.ACTION_VIEW, uri))
                } catch (_: ActivityNotFoundException) {
                    // The bundled privacy policy and license remain readable offline.
                }
            }
            return true
        }

        private fun isApplicationAsset(uri: Uri): Boolean {
            return uri.scheme == "https" && uri.host == ASSET_HOST
        }

        private fun isProjectLink(uri: Uri): Boolean {
            if (uri.scheme != "https" || uri.encodedAuthority != "github.com" || uri.query != null) {
                return false
            }
            return uri.encodedPath in setOf(
                "/chubbyk-uu/rust-gomoku/tree/v${BuildConfig.VERSION_NAME}",
                "/chubbyk-uu/rust-gomoku/blob/master/docs/privacy-policy.md",
                "/chubbyk-uu/rust-gomoku/issues/new",
            )
        }
    }

    /**
     * Narrow JavaScript bridge. The page calls [request] with a JSON command and
     * a caller-generated callback id; the JSON response is delivered back to
     * `window.__onNativeResult(callbackId, responseJson)` on the UI thread.
     *
     * `@JavascriptInterface` methods run on a private WebView thread, so all
     * WebView access is marshalled back to the main thread by the ViewModel's
     * main-thread callback.
     */
    private class WebAppBridge(
        private val viewModel: GameViewModel,
        private val webView: WebView,
    ) {
        @Volatile
        private var active = true

        fun release() {
            active = false
        }

        @JavascriptInterface
        fun request(requestJson: String, callbackId: String) {
            viewModel.request(requestJson) { response ->
                if (!active) {
                    return@request
                }
                val args = "${JSONObject.quote(callbackId)}, ${JSONObject.quote(response)}"
                try {
                    webView.evaluateJavascript("window.__onNativeResult($args);", null)
                } catch (ignored: Exception) {
                    // WebView torn down between the active check and dispatch.
                }
            }
        }
    }
}
