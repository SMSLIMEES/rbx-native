package dev.rbxnative.viewer

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.ConsoleMessage
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Toast
import java.io.File
import java.io.IOException

/**
 * rbxview on Android: the browser build of the viewer (the same
 * `crates/rbx_viewer/web` page, wasm and all) in a WebView, with the two
 * things a page cannot do for itself supplied by the app: a Storage Access
 * Framework picker instead of a file input, and Roblox's assets resolved by
 * [AssetProxy] through [LocalServer] instead of `rbxview --serve`.
 *
 * The page is loaded over loopback HTTP (`http://127.0.0.1:<port>/`) rather
 * than from `file://` for two reasons: `WebAssembly.instantiateStreaming`
 * and ES modules want a real origin, and the same URL can be handed to
 * Chrome, which ships WebGPU on every Pixel 7 while the system WebView's
 * support is still patchy. "Open in Chrome" is the escape hatch when the
 * WebView reports no `navigator.gpu`; the page says so itself.
 */
class MainActivity : Activity() {
    private lateinit var webView: WebView
    private lateinit var proxy: AssetProxy
    private lateinit var server: LocalServer

    private var port = 0

    /** A pick or an intent that arrived before the page finished loading. */
    private var pageReady = false
    private var pendingRefresh = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        proxy = AssetProxy(this)
        server = LocalServer(this, proxy)
        port = server.start() ?: 0
        // Rotation recreates the activity but not the file: the place the
        // user opened before is still at `/place`, so put it back.
        if (savedInstanceState?.getBoolean(STATE_HAS_PLACE) == true) {
            val target = File(File(cacheDir, "place"), "current")
            if (target.isFile && target.length() > 0) {
                server.place = target
                server.placeName = savedInstanceState.getString(STATE_PLACE_NAME) ?: "place.rbxl"
            }
        }

        webView = WebView(this).apply {
            setBackgroundColor(0xFF1B1B1F.toInt())
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        setContentView(webView)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            // The page is a full-window canvas handling its own gestures.
            setSupportZoom(false)
            builtInZoomControls = false
            displayZoomControls = false
            mediaPlaybackRequiresUserGesture = false
            allowFileAccess = false
            allowContentAccess = false
            // Loopback is cleartext; nothing else may be.
            mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
            userAgentString = "$userAgentString rbx-native-android/${versionName()}"
        }

        WebView.setWebContentsDebuggingEnabled(
            (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
        )

        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: WebResourceRequest,
            ): Boolean = externalIfOffLoopback(request.url)

            override fun onPageFinished(view: WebView, url: String?) {
                pageReady = true
                if (pendingRefresh) {
                    pendingRefresh = false
                    refresh()
                }
            }
        }
        webView.webChromeClient = object : WebChromeClient() {
            override fun onConsoleMessage(message: ConsoleMessage): Boolean {
                Log.d(TAG, "page: ${message.message()} (${message.sourceId()}:${message.lineNumber()})")
                return true
            }
        }
        webView.addJavascriptInterface(Bridge(), "Android")

        if (port == 0) {
            showFatal("Could not start the local server")
        } else {
            Log.i(TAG, "serving on http://127.0.0.1:$port/ (WebView ${webViewVersion()})")
            webView.loadUrl("http://127.0.0.1:$port/")
        }

        consumeIntent(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(STATE_HAS_PLACE, server.place != null)
        outState.putString(STATE_PLACE_NAME, server.placeName)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        consumeIntent(intent)
    }

    override fun onDestroy() {
        server.stop()
        webView.destroy()
        super.onDestroy()
    }

    // ---- opening places ---------------------------------------------------

    /** What the page can reach in the app: the picker and the Chrome fallback. */
    private inner class Bridge {
        @JavascriptInterface
        fun pickPlace() = runOnUiThread { pick() }

        /**
         * WebGPU lives in the browser, not the page: the system WebView does
         * not always expose it, Chrome on the same phone does. The page calls
         * this when it finds no `navigator.gpu`.
         */
        @JavascriptInterface
        fun openInBrowser() = runOnUiThread {
            if (port == 0) return@runOnUiThread
            val url = "http://127.0.0.1:$port/"
            val chrome = Intent(Intent.ACTION_VIEW, Uri.parse(url)).apply {
                setPackage("com.android.chrome")
            }
            try {
                startActivity(chrome)
            } catch (err: ActivityNotFoundException) {
                try {
                    startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                } catch (err2: ActivityNotFoundException) {
                    Toast.makeText(
                        this@MainActivity,
                        "No browser to open $url in",
                        Toast.LENGTH_LONG,
                    ).show()
                }
            }
        }

        @JavascriptInterface
        fun hasPlace(): Boolean = server.place != null
    }

    private fun pick() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            // `.rbxl`/`.rbxm` have no registered type on most devices, so the
            // picker is opened wide; the place is sniffed by the viewer either
            // way (binary or XML).
            type = "*/*"
        }
        startPicker(intent)
    }

    /**
     * `startActivityForResult`, deprecated but the only contract available
     * without an AndroidX dependency. A ROM with no document picker at all
     * gets a toast rather than a crash.
     */
    @Suppress("DEPRECATION")
    private fun startPicker(intent: Intent) {
        try {
            startActivityForResult(intent, REQUEST_PICK)
        } catch (err: Exception) {
            Log.w(TAG, "no document picker", err)
            Toast.makeText(this, "No file picker on this device", Toast.LENGTH_LONG).show()
        }
    }

    @Deprecated("The platform contract, kept to avoid an androidx.activity dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_PICK || resultCode != RESULT_OK) return
        data?.data?.let(::openPlace)
    }

    private fun consumeIntent(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        intent.data?.let(::openPlace)
    }

    /** Copies `uri` next to the app, then asks the page to reopen `/place`. */
    private fun openPlace(uri: Uri) {
        val name = displayName(uri) ?: "place.rbxl"
        Toast.makeText(this, "Opening $name…", Toast.LENGTH_SHORT).show()
        Thread {
            try {
                val dir = File(cacheDir, "place").apply { mkdirs() }
                val target = File(dir, "current")
                contentResolver.openInputStream(uri)?.use { input ->
                    target.outputStream().use { output -> input.copyTo(output, 1 shl 20) }
                } ?: throw IOException("cannot open $uri")
                if (target.length() == 0L) throw IOException("$name is empty")
                server.place = target
                server.placeName = name
                runOnUiThread {
                    if (pageReady) refresh() else pendingRefresh = true
                }
            } catch (err: Exception) {
                Log.w(TAG, "cannot open $uri", err)
                runOnUiThread {
                    Toast.makeText(
                        this@MainActivity,
                        "Cannot open $name: ${err.message}",
                        Toast.LENGTH_LONG,
                    ).show()
                }
            }
        }.apply { isDaemon = true }.start()
    }

    private fun refresh() {
        webView.evaluateJavascript("window.refreshPlace?.()", null)
    }

    private fun displayName(uri: Uri): String? {
        if (uri.scheme == "file") return uri.lastPathSegment
        val columns = arrayOf(android.provider.OpenableColumns.DISPLAY_NAME)
        return contentResolver.query(uri, columns, null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }
    }

    // ---- miscellaneous ----------------------------------------------------

    /** Anything that is not our own loopback page goes to the real browser. */
    private fun externalIfOffLoopback(uri: Uri): Boolean {
        if (uri.host == "127.0.0.1" && uri.port == port) return false
        return try {
            startActivity(Intent(Intent.ACTION_VIEW, uri))
            true
        } catch (err: ActivityNotFoundException) {
            false
        }
    }

    private fun showFatal(message: String) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
        Log.e(TAG, message)
        // The page cannot render without the server, so say so in the view.
        val fallback = android.widget.TextView(this).apply {
            text = message
            setPadding(48, 96, 48, 48)
        }
        setContentView(fallback)
    }

    /**
     * Which WebView is running matters: WebGPU arrived in Chrome 121, and
     * whether the WebView build on this device exposes it is exactly what the
     * user needs to know if the page cannot start.
     */
    private fun webViewVersion(): String =
        WebView.getCurrentWebViewPackage()?.let {
            "${it.packageName} ${it.versionName}"
        } ?: "unknown"

    private fun versionName(): String = try {
        packageManager.getPackageInfo(packageName, 0).versionName ?: "dev"
    } catch (_: Exception) {
        "dev"
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) webView.requestFocus()
    }

    companion object {
        private const val TAG = "rbxview"
        private const val REQUEST_PICK = 1
        private const val STATE_HAS_PLACE = "has-place"
        private const val STATE_PLACE_NAME = "place-name"
    }
}
