package dev.rbxnative.viewer

import android.content.Context
import java.io.BufferedOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.URLDecoder
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * `rbxview --serve` on a phone: the viewer page, its wasm module and Roblox's
 * assets, over plain HTTP on `127.0.0.1`, until the app closes.
 *
 * The desktop has this process in Rust (`crates/rbx_viewer/src/serve.rs`); a
 * page cannot reach Roblox's CDNs itself, they send no CORS headers. Here it
 * is a handful of sockets instead, so the very page and the very fetcher code
 * work unchanged, and — the reason it is a real server rather than
 * `shouldInterceptRequest` — the same URL can be handed to Chrome, whose
 * WebGPU support is wider than the system WebView's.
 *
 * The routes are `serve.rs`'s:
 *  - `/asset/id/<n>`, `/asset/native/<path>` — through [AssetProxy];
 *  - `/place` — the file the user opened, if any;
 *  - everything else — a file from the APK's `assets/web`, `/` being
 *    `index.html`.
 */
class LocalServer(
    private val context: Context,
    private val proxy: AssetProxy,
) {
    /** The place served at `/place`; written on the UI thread, read on ours. */
    @Volatile var place: File? = null
    @Volatile var placeName: String = "place"

    private var socket: ServerSocket? = null
    private var acceptor: Thread? = null
    /** Recreated by `start`, so the activity can be recreated without one. */
    private var workers = pools()

    /** Binds to a free loopback port and starts answering; `null` on failure. */
    fun start(): Int? {
        if (socket != null) return socket?.localPort
        workers = pools()
        return try {
            val bound = ServerSocket(0, BACKLOG, InetAddress.getByName("127.0.0.1"))
            socket = bound
            acceptor = Thread({ acceptLoop(bound) }, "rbxview-accept").apply {
                isDaemon = true
                start()
            }
            bound.localPort
        } catch (err: IOException) {
            proxy.log("cannot listen on loopback: ${err.message}")
            null
        }
    }

    private fun pools() = Executors.newFixedThreadPool(CONNECTIONS) { runnable ->
        Thread(runnable, "rbxview-http").apply { isDaemon = true }
    }

    fun stop() {
        try {
            socket?.close()
        } catch (_: IOException) {
        }
        socket = null
        workers.shutdown()
        try {
            workers.awaitTermination(1, TimeUnit.SECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    private fun acceptLoop(bound: ServerSocket) {
        while (!bound.isClosed) {
            val client = try {
                bound.accept()
            } catch (err: IOException) {
                if (bound.isClosed) return else continue
            }
            try {
                workers.execute { client.use(::answer) }
            } catch (err: Exception) {
                client.close()
            }
        }
    }

    private fun answer(client: Socket) {
        try {
            client.soTimeout = SOCKET_TIMEOUT_MS
            val input = client.getInputStream().buffered()
            val parts = readHead(input) ?: return
            val method = parts.firstOrNull() ?: return
            if (method != "GET" && method != "HEAD") {
                respond(client, Reply(405, "text/plain", "GET only".toByteArray()))
                return
            }
            val target = parts.getOrNull(1)?.substringBefore('?') ?: return
            val reply = route(target)
            respond(client, reply, headOnly = method == "HEAD")
        } catch (err: Exception) {
            // A page that navigated away, or a socket closed mid-write: the
            // next request gets a fresh connection, there is nothing to retry.
            if (err !is IOException) proxy.log("request failed: ${err.message}")
        }
    }

    /**
     * The request line's three words, with the headers read off the socket
     * and discarded: this server acts on nothing in them, but they have to be
     * gone from the stream before the answer goes out.
     */
    private fun readHead(input: InputStream): List<String>? {
        val requestLine = readLine(input) ?: return null
        while (true) {
            val header = readLine(input) ?: return requestLine.split(' ')
            if (header.isEmpty()) return requestLine.split(' ')
        }
    }

    /** One CRLF-terminated line, `null` at end of stream or past the cap. */
    private fun readLine(input: InputStream): String? {
        val line = StringBuilder()
        while (line.length < MAX_HEAD_BYTES) {
            val byte = input.read()
            if (byte < 0) return null
            if (byte == '\n'.code) return line.toString().trimEnd('\r')
            line.append(byte.toChar())
        }
        return null
    }

    private data class Reply(
        val status: Int,
        val kind: String,
        val body: ByteArray,
        val headers: Map<String, String> = emptyMap(),
    )

    private fun route(target: String): Reply = when {
        target.startsWith("/asset/") -> {
            val answer = proxy.resolve(target.removePrefix("/asset/"))
            Reply(
                answer.status,
                "application/octet-stream",
                answer.body,
            )
        }
        target == "/place" -> placeReply()
        else -> staticReply(target)
    }

    private fun placeReply(): Reply {
        val file = place ?: return Reply(404, "text/plain", "no place".toByteArray())
        return try {
            Reply(
                200, "application/octet-stream", file.readBytes(),
                // The page shows the file name it came from. Percent-encoded:
                // a header is ASCII-only, and a name may well not be.
                mapOf("X-Place-Name" to encodeHeader(placeName)),
            )
        } catch (err: IOException) {
            Reply(404, "text/plain", "cannot read ${file.name}: ${err.message}".toByteArray())
        }
    }

    private fun staticReply(target: String): Reply {
        val relative = URLDecoder.decode(target, "UTF-8").trimStart('/').ifEmpty { "index.html" }
        // Anything trying to climb out of `assets/web` is not found, as in
        // `serve.rs::file`.
        if (relative.split('/').any { it == ".." || it == "." || it.isEmpty() }) {
            return Reply(404, "text/plain", "not found".toByteArray())
        }
        val bytes = try {
            context.assets.open("web/$relative").use { it.readBytes() }
        } catch (err: IOException) {
            return Reply(404, "text/plain", "not found".toByteArray())
        }
        return Reply(200, mediaType(relative), bytes)
    }

    /** `encodeURIComponent`'s escaping, which the page decodes. */
    private fun encodeHeader(text: String): String {
        val out = StringBuilder()
        for (byte in text.toByteArray(Charsets.UTF_8)) {
            val value = byte.toInt() and 0xFF
            val safe = value in 'a'.code..'z'.code || value in 'A'.code..'Z'.code ||
                value in '0'.code..'9'.code || value.toChar() in "-_.!~*'()"
            if (safe) {
                out.append(value.toChar())
            } else {
                out.append('%').append(HEX[value shr 4]).append(HEX[value and 0x0F])
            }
        }
        return out.toString()
    }

    private fun mediaType(relative: String): String =
        when (relative.substringAfterLast('.', "")) {
            "html" -> "text/html; charset=utf-8"
            "js", "mjs" -> "text/javascript"
            // Required as is for `WebAssembly.instantiateStreaming`.
            "wasm" -> "application/wasm"
            "css" -> "text/css"
            "json" -> "application/json"
            "png" -> "image/png"
            "svg" -> "image/svg+xml"
            "ico" -> "image/x-icon"
            else -> "application/octet-stream"
        }

    private fun respond(client: Socket, reply: Reply, headOnly: Boolean = false) {
        val reason = when (reply.status) {
            200 -> "OK"
            404 -> "Not Found"
            405 -> "Method Not Allowed"
            503 -> "Service Unavailable"
            else -> "Error"
        }
        val output = BufferedOutputStream(client.getOutputStream())
        val head = buildString {
            append("HTTP/1.1 ${reply.status} $reason\r\n")
            append("Content-Type: ${reply.kind}\r\n")
            append("Content-Length: ${reply.body.size}\r\n")
            // `/place` changes under the same URL; the assets ship with the
            // APK, but a new build means a new page anyway.
            append("Cache-Control: no-store\r\n")
            for ((name, value) in reply.headers) append("$name: $value\r\n")
            append("Connection: close\r\n\r\n")
        }
        output.write(head.toByteArray(Charsets.US_ASCII))
        if (!headOnly) output.write(reply.body)
        output.flush()
    }

    companion object {
        private const val HEX = "0123456789ABCDEF"

        /** `assets::WORKERS` on the desktop, and a browser's per-host cap. */
        private const val CONNECTIONS = 6
        private const val BACKLOG = 24
        private const val SOCKET_TIMEOUT_MS = 120_000
        private const val MAX_HEAD_BYTES = 16 * 1024
    }
}
