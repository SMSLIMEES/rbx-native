package dev.rbxnative.explorer

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.util.Base64
import android.util.Log
import android.view.View
import android.view.WindowInsetsController
import android.webkit.ConsoleMessage
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.FileNotFoundException
import java.io.InputStream
import java.util.concurrent.Executors

/**
 * RBX Explorer: the wasm build of `crates/rbx_explorer` in a WebView.
 *
 * The page does everything a user sees — the tree, the property sheet, the
 * script editor — and this activity does what a page can't:
 *  - serves the page out of the APK under an https origin (intercepted, so a
 *    secure context with no server and no network), plus the open file at
 *    `/doc/current`;
 *  - opens files with the system picker or from an "open with" intent, and
 *    writes them back (or to a new file) through the Storage Access
 *    Framework;
 *  - keeps the recent-file list, tints the system bars to the page's theme,
 *    and turns the back gesture into the page's own back.
 *
 * The bridge the page sees as `window.Android` is [Bridge]; the page's side
 * of the contract is `crates/rbx_explorer/web/index.html`.
 */
class MainActivity : Activity() {
    private lateinit var web: WebView
    private val io = Executors.newSingleThreadExecutor()

    /** The bytes `/doc/current` serves: the file being opened right now. */
    @Volatile
    private var current: ByteArray? = null

    /** Where the open document lives, if it can be written back there. */
    private var currentUri: Uri? = null

    /** A file that arrived before the page finished loading. */
    private var pendingUri: Uri? = null
    private var pageReady = false

    /** The save being streamed in from the page, chunk by chunk. */
    private var saveBuffer = ByteArrayOutputStream()
    private var saveBytes: ByteArray? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // chrome://inspect for debug builds only.
        WebView.setWebContentsDebuggingEnabled(
            applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0,
        )
        web = WebView(this)
        web.setBackgroundColor(Color.TRANSPARENT)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            mediaPlaybackRequiresUserGesture = true
            cacheMode = WebSettings.LOAD_NO_CACHE
            // The page sizes its own text (and has its own code zoom); the
            // system font scale would break the fixed 48 dp rows.
            textZoom = 100
            setSupportZoom(false)
        }
        web.addJavascriptInterface(Bridge(), "Android")
        web.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(
                view: WebView,
                request: WebResourceRequest,
            ): WebResourceResponse? = serve(request.url)

            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val url = request.url
                if (url.host == HOST) return false
                // A link out of the page (there are none today) goes to the browser.
                runCatching { startActivity(Intent(Intent.ACTION_VIEW, url)) }
                return true
            }
        }
        web.webChromeClient = object : WebChromeClient() {
            override fun onConsoleMessage(message: ConsoleMessage): Boolean {
                Log.i(TAG, "page: ${message.message()} (${message.sourceId()}:${message.lineNumber()})")
                return true
            }
        }
        setContentView(web)
        pendingUri = intentUri(intent)
        web.loadUrl("https://$HOST/web/index.html")
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val uri = intentUri(intent) ?: return
        if (pageReady) openUri(uri, persist = false) else pendingUri = uri
    }

    @Deprecated("The page decides what back means; see rbx_back.")
    override fun onBackPressed() {
        if (!pageReady) {
            @Suppress("DEPRECATION")
            super.onBackPressed()
            return
        }
        web.evaluateJavascript("window.rbxBack ? window.rbxBack() : false") { handled ->
            if (handled != "true" && !isFinishing) finish()
        }
    }

    override fun onDestroy() {
        io.shutdown()
        web.destroy()
        super.onDestroy()
    }

    @Deprecated("Activity results for the picker and \"save as\".")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data
        when (requestCode) {
            REQUEST_OPEN -> {
                if (resultCode == RESULT_OK && uri != null) openUri(uri, persist = true)
            }
            REQUEST_CREATE -> {
                val bytes = saveBytes
                saveBytes = null
                if (resultCode != RESULT_OK || uri == null || bytes == null) {
                    js("rbxSaved(false, '', '', false)")
                    return
                }
                takePermission(uri, write = true)
                io.execute {
                    val result = runCatching { write(uri, bytes) }
                    val name = displayName(uri) ?: "file"
                    runOnUiThread {
                        result.fold(
                            onSuccess = {
                                currentUri = uri
                                remember(uri, name, bytes.size.toLong())
                                js("rbxSaved(true, ${quote(name)}, '', true)")
                            },
                            onFailure = {
                                Log.w(TAG, "save as $uri", it)
                                js("rbxSaved(false, '', ${quote(it.message ?: it.toString())}, false)")
                            },
                        )
                    }
                }
            }
        }
    }

    // ---- serving the page ------------------------------------------------

    private fun serve(url: Uri): WebResourceResponse? {
        if (url.host != HOST) return null
        val path = url.path ?: return notFound()
        if (path == "/doc/current") {
            val bytes = current ?: return notFound()
            return response("application/octet-stream", ByteArrayInputStream(bytes))
        }
        val asset = path.trimStart('/')
        if (!asset.startsWith("web/") || asset.contains("..")) return notFound()
        return try {
            response(mimeFor(asset), assets.open(asset))
        } catch (_: FileNotFoundException) {
            notFound()
        }
    }

    private fun response(mime: String, body: InputStream): WebResourceResponse =
        WebResourceResponse(mime, null, 200, "OK", mapOf("Cache-Control" to "no-store"), body)

    private fun notFound(): WebResourceResponse =
        WebResourceResponse("text/plain", "utf-8", 404, "Not Found", emptyMap(), ByteArrayInputStream(ByteArray(0)))

    private fun mimeFor(path: String): String = when (path.substringAfterLast('.', "")) {
        "html" -> "text/html"
        "js", "mjs" -> "text/javascript"
        "css" -> "text/css"
        // `instantiateStreaming` refuses anything else.
        "wasm" -> "application/wasm"
        "ttf" -> "font/ttf"
        "svg" -> "image/svg+xml"
        "json" -> "application/json"
        else -> "application/octet-stream"
    }

    // ---- opening files ---------------------------------------------------

    @Suppress("DEPRECATION")
    private fun intentUri(intent: Intent?): Uri? {
        intent ?: return null
        return when (intent.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND -> intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri
            else -> null
        }
    }

    /**
     * Reads [uri] off the main thread and hands it to the page.
     * [persist]: the picker's grant can outlive this process, so the file
     * goes into the recent list; an "open with" grant usually can't.
     */
    private fun openUri(uri: Uri, persist: Boolean) {
        val kept = if (persist) takePermission(uri, write = true) else tryPersist(uri)
        val name = displayName(uri) ?: uri.lastPathSegment?.substringAfterLast('/') ?: "file"
        js("rbxLoading(${quote(name)})")
        io.execute {
            val result = runCatching { read(uri) }
            runOnUiThread {
                result.fold(
                    onSuccess = { bytes ->
                        current = bytes
                        val writable = canWrite(uri)
                        currentUri = if (writable) uri else null
                        if (kept) remember(uri, name, bytes.size.toLong())
                        js("rbxOpenFromHost(${quote(name)}, $writable)")
                    },
                    onFailure = {
                        Log.w(TAG, "open $uri", it)
                        js("rbxOpenFailed(${quote(it.message ?: it.toString())})")
                    },
                )
            }
        }
    }

    private fun read(uri: Uri): ByteArray =
        contentResolver.openInputStream(uri)?.use { it.readBytes() }
            ?: throw FileNotFoundException(uri.toString())

    /**
     * Replaces the file's contents. `"wt"` is the documented truncating
     * mode; some providers only know `"w"` (which on old ones does not
     * truncate, so it is the last resort).
     */
    private fun write(uri: Uri, bytes: ByteArray) {
        var last: Exception? = null
        for (mode in arrayOf("wt", "rwt", "w")) {
            try {
                val stream = contentResolver.openOutputStream(uri, mode) ?: continue
                stream.use { it.write(bytes) }
                return
            } catch (err: IllegalArgumentException) {
                last = err
            } catch (err: FileNotFoundException) {
                last = err
            } catch (err: UnsupportedOperationException) {
                last = err
            }
        }
        throw last ?: FileNotFoundException(uri.toString())
    }

    private fun takePermission(uri: Uri, write: Boolean): Boolean {
        val read = Intent.FLAG_GRANT_READ_URI_PERMISSION
        val both = read or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        if (write && runCatching { contentResolver.takePersistableUriPermission(uri, both) }.isSuccess) return true
        return runCatching { contentResolver.takePersistableUriPermission(uri, read) }.isSuccess
    }

    private fun tryPersist(uri: Uri): Boolean = uri.scheme == "content" && takePermission(uri, write = true)

    /** Whether a plain "save" may write back over [uri]. */
    private fun canWrite(uri: Uri): Boolean {
        if (uri.scheme != "content") return false
        val granted = checkUriPermission(
            uri, Process.myPid(), Process.myUid(), Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) return false
        if (!DocumentsContract.isDocumentUri(this, uri)) return true
        return runCatching {
            contentResolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_FLAGS), null, null, null)?.use {
                it.moveToFirst() && (it.getInt(0) and DocumentsContract.Document.FLAG_SUPPORTS_WRITE) != 0
            } ?: false
        }.getOrDefault(false)
    }

    private fun displayName(uri: Uri): String? = runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull()

    // ---- recents ---------------------------------------------------------

    private val prefs by lazy { getSharedPreferences("explorer", MODE_PRIVATE) }

    private fun recentList(): JSONArray =
        runCatching { JSONArray(prefs.getString("recents", "[]")) }.getOrDefault(JSONArray())

    private fun remember(uri: Uri, name: String, size: Long) {
        val id = uri.toString()
        val next = JSONArray()
        next.put(
            JSONObject()
                .put("id", id)
                .put("name", name)
                .put("size", size)
                .put("time", System.currentTimeMillis()),
        )
        val old = recentList()
        for (i in 0 until old.length()) {
            val item = old.optJSONObject(i) ?: continue
            if (item.optString("id") != id && next.length() < MAX_RECENTS) next.put(item)
        }
        prefs.edit().putString("recents", next.toString()).apply()
        js("rbxRecentsChanged()")
    }

    private fun forget(id: String) {
        val old = recentList()
        val next = JSONArray()
        for (i in 0 until old.length()) {
            val item = old.optJSONObject(i) ?: continue
            if (item.optString("id") != id) next.put(item)
        }
        prefs.edit().putString("recents", next.toString()).apply()
        runCatching {
            contentResolver.releasePersistableUriPermission(
                Uri.parse(id),
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        }
        js("rbxRecentsChanged()")
    }

    // ---- system bars -----------------------------------------------------

    private fun setBars(dark: Boolean) {
        val surface = Color.parseColor(if (dark) "#10140f" else "#f7fbf1")
        @Suppress("DEPRECATION")
        window.statusBarColor = surface
        @Suppress("DEPRECATION")
        window.navigationBarColor = surface
        web.setBackgroundColor(surface)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
            window.insetsController?.setSystemBarsAppearance(if (dark) 0 else light, light)
        } else {
            @Suppress("DEPRECATION")
            val flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            @Suppress("DEPRECATION")
            val now = window.decorView.systemUiVisibility
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility = if (dark) now and flags.inv() else now or flags
        }
    }

    // ---- page calls ------------------------------------------------------

    private fun js(code: String) {
        runOnUiThread { if (pageReady) web.evaluateJavascript(code, null) }
    }

    private fun quote(text: String): String = JSONObject.quote(text)

    /**
     * `window.Android`. Called on a WebView binder thread, in the order the
     * page makes the calls; anything touching the UI posts to it.
     */
    inner class Bridge {
        @JavascriptInterface
        fun ready() = runOnUiThread {
            pageReady = true
            pendingUri?.let { openUri(it, persist = false) }
            pendingUri = null
        }

        @JavascriptInterface
        fun pick() = runOnUiThread {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                // Roblox files have no MIME type of their own: show everything.
                type = "*/*"
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
                )
            }
            @Suppress("DEPRECATION")
            runCatching { startActivityForResult(intent, REQUEST_OPEN) }
                .onFailure { js("rbxOpenFailed(${quote("Нет приложения для выбора файлов")})") }
        }

        @JavascriptInterface
        fun saveBegin() {
            saveBuffer = ByteArrayOutputStream()
        }

        @JavascriptInterface
        fun saveChunk(base64: String) {
            saveBuffer.write(Base64.decode(base64, Base64.DEFAULT))
        }

        @JavascriptInterface
        fun saveCommit(name: String, overwrite: Boolean) {
            val bytes = saveBuffer.toByteArray()
            saveBuffer = ByteArrayOutputStream()
            runOnUiThread {
                val target = currentUri
                if (overwrite && target != null) {
                    io.execute {
                        val result = runCatching { write(target, bytes) }
                        runOnUiThread {
                            result.fold(
                                onSuccess = {
                                    remember(target, name, bytes.size.toLong())
                                    js("rbxSaved(true, ${quote(name)}, '', true)")
                                },
                                onFailure = {
                                    Log.w(TAG, "save $target", it)
                                    js("rbxSaved(false, '', ${quote(it.message ?: it.toString())}, false)")
                                },
                            )
                        }
                    }
                    return@runOnUiThread
                }
                saveBytes = bytes
                val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = if (name.endsWith("x")) "application/xml" else "application/octet-stream"
                    putExtra(Intent.EXTRA_TITLE, name)
                }
                @Suppress("DEPRECATION")
                runCatching { startActivityForResult(intent, REQUEST_CREATE) }.onFailure {
                    saveBytes = null
                    js("rbxSaved(false, '', ${quote("Нет приложения для сохранения файлов")}, false)")
                }
            }
        }

        @JavascriptInterface
        fun recents(): String = recentList().toString()

        @JavascriptInterface
        fun openRecent(id: String) = runOnUiThread {
            val uri = Uri.parse(id)
            val held = contentResolver.persistedUriPermissions.any { it.uri == uri && it.isReadPermission }
            if (!held) {
                forget(id)
                js("rbxOpenFailed(${quote("Нет доступа к файлу — откройте его заново")})")
                return@runOnUiThread
            }
            val name = displayName(uri)
            if (name == null) {
                forget(id)
                js("rbxOpenFailed(${quote("Файл больше не найден")})")
                return@runOnUiThread
            }
            openUri(uri, persist = true)
        }

        @JavascriptInterface
        fun setBars(dark: Boolean) = runOnUiThread { this@MainActivity.setBars(dark) }

        @JavascriptInterface
        fun copy(text: String) = runOnUiThread {
            val clipboard = getSystemService(ClipboardManager::class.java)
            clipboard?.setPrimaryClip(ClipData.newPlainText("RBX Explorer", text))
        }

        @JavascriptInterface
        fun exit() = runOnUiThread { if (!isFinishing) finish() }
    }

    companion object {
        private const val TAG = "RbxExplorer"

        /** The reserved WebView asset host: never resolves on the network. */
        private const val HOST = "appassets.androidplatform.net"
        private const val REQUEST_OPEN = 1
        private const val REQUEST_CREATE = 2
        private const val MAX_RECENTS = 12
    }
}
