package dev.rbxnative.viewer

import android.content.Context
import android.util.Log
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap
import java.util.zip.ZipFile

/**
 * The app's own `rbxview --serve` (see `crates/rbx_viewer/src/serve.rs`).
 *
 * The page cannot fetch from Roblox's CDNs itself: they send no CORS headers.
 * `rbxview --serve` resolves the bytes on the desktop; on a phone this class
 * does the same job for the WebView, which reaches it through
 * `shouldInterceptRequest` on the fake `appassets.androidplatform.net` origin.
 *
 * Two routes, exactly the ones `load::fetcher::web` asks for:
 *  - `asset/id/<n>`     — `https://assetdelivery.roblox.com/v1/asset?id=<n>`,
 *                         anonymous (no Open Cloud key on a phone).
 *  - `asset/native/<p>` — a file out of Studio's `rbxasset://` content
 *                         packages on `setup.rbxcdn.com`.
 *
 * Statuses carry the resolver's verdict, as the Rust server's do: 404 for a
 * failure asking again cannot change, 503 for one of the moment (the page
 * retries those, see `load::Resident`).
 */
class AssetProxy(context: Context) {
    class Answer(val status: Int, val body: ByteArray)

    /** Diagnostics, in logcat and on the page's own warnings list. */
    fun log(message: String) = Log.i(TAG, message)

    private val root = File(context.cacheDir, "rbx-assets")
    private val ids = File(root, "id")
    private val packages = File(root, "native-packages")
    private val versionFile = File(root, "studio-version")
    /** One download per package, however many assets ask at once. */
    private val locks = ConcurrentHashMap<String, Any>()

    /** A file the CDN says is simply not there: asking again will not help. */
    private class Missing(message: String) : Exception(message)

    fun resolve(reference: String): Answer {
        val (kind, rest) = reference.split("/", limit = 2)
            .let { if (it.size == 2) it[0] to it[1] else "" to "" }
        return when (kind) {
            "id" -> {
                val id = rest.toLongOrNull()
                if (id == null) Answer(404, "not an asset id: $rest".toByteArray())
                else byId(id)
            }
            "native" -> {
                val path = percentDecode(rest)
                if (path.isEmpty()) Answer(404, "not an asset path".toByteArray())
                else byNative(path)
            }
            else -> Answer(404, "not an asset reference".toByteArray())
        }
    }

    // ---- /asset/id/<n> ---------------------------------------------------

    private fun byId(id: Long): Answer {
        val cached = File(ids, id.toString())
        if (cached.isFile && cached.length() > 0) return Answer(200, cached.readBytes())
        return try {
            val bytes = delivery(id)
            store(cached, bytes)
            Answer(200, bytes)
        } catch (err: Missing) {
            Answer(404, "${err.message}".toByteArray())
        } catch (err: IOException) {
            Answer(503, "asset $id: ${err.message ?: "network error"}".toByteArray())
        }
    }

    private fun delivery(id: Long): ByteArray {
        val url = "https://assetdelivery.roblox.com/v1/asset?id=$id"
        // The redirect is followed by hand so the first hop's status stays
        // visible: a 401/403 there is the "this asset needs a sign-in" answer,
        // which no retry will change (see `Client::asset_anonymous`).
        return connect(url, follow = false).use { first ->
            val code = first.responseCode
            if (code in 300..399) {
                val location = first.getHeaderField("Location")
                    ?: throw IOException("redirect with no Location")
                return@use get(location)
            }
            if (code == 401 || code == 403) throw Missing("asset $id needs a Roblox sign-in")
            if (code == 429 || code >= 500) throw IOException("asset delivery said HTTP $code")
            if (code !in 200..299) throw Missing("asset $id: no such asset (HTTP $code)")
            readCapped(first.inputStream, MAX_ASSET_BYTES)
        }
    }

    private fun get(url: String): ByteArray = connect(url, follow = true).use { connection ->
        val code = connection.responseCode
        if (code == 401 || code == 403) throw Missing("asset refused by the CDN (HTTP $code)")
        if (code !in 200..299) throw IOException("asset CDN said HTTP $code")
        readCapped(connection.inputStream, MAX_ASSET_BYTES)
    }

    // ---- /asset/native/<path> -------------------------------------------

    private fun byNative(path: String): Answer {
        substitutes[path.lowercase()]?.let { return byId(it) }
        val candidates = candidatesFor(path)
            ?: return Answer(404, "rbxasset://$path is not in any content package".toByteArray())
        return try {
            Answer(200, fromPackages(path, candidates))
        } catch (err: Missing) {
            Answer(404, "rbxasset://$path: ${err.message}".toByteArray())
        } catch (err: IOException) {
            Answer(503, "rbxasset://$path: ${err.message ?: "network error"}".toByteArray())
        }
    }

    private fun fromPackages(path: String, candidates: List<String>): ByteArray {
        val version = studioVersion()
        for (packageName in candidates) {
            val bytes = extract(packageZip(version, packageName), path)
            if (bytes != null) return bytes
        }
        throw Missing("not in ${candidates.joinToString()}")
    }

    private fun packageZip(version: String, packageName: String): File {
        val file = File(packages, "$version-$packageName")
        if (file.isFile && file.length() > 0) return file
        val lock = locks.computeIfAbsent(file.name) { Any() }
        synchronized(lock) {
            if (file.isFile && file.length() > 0) return file
            val url = "https://setup.rbxcdn.com/$version-$packageName"
            connect(url, follow = true).use { connection ->
                val code = connection.responseCode
                if (code !in 200..299) throw IOException("$packageName: HTTP $code")
                val declared = connection.getHeaderField("Content-Length")?.toLongOrNull() ?: -1
                if (declared > MAX_PACKAGE_BYTES) {
                    throw IOException("$packageName is ${declared / 1024 / 1024} MiB, refusing")
                }
                val temp = File(file.parentFile, "${file.name}.tmp-${Thread.currentThread().id}")
                temp.outputStream().use { output ->
                    val copied = copyCapped(connection.inputStream, output, MAX_PACKAGE_BYTES)
                    if (copied <= 0) throw IOException("$packageName came back empty")
                }
                if (!temp.renameTo(file)) {
                    temp.delete()
                    throw IOException("could not keep $packageName")
                }
            }
        }
        return file
    }

    private fun extract(zip: File, path: String): ByteArray? {
        // Studio's packages are rooted at the rbxasset directory and use
        // backslashes; the same fallbacks the Rust extractor tries.
        val remainder = path.split("/", limit = 2).lastOrNull() ?: path
        val names = listOf(
            remainder.replace('/', '\\'),
            remainder,
            path,
            path.replace('/', '\\'),
            "content/$path",
        )
        val found = try {
            ZipFile(zip).use { archive ->
                var bytes: ByteArray? = null
                for (name in names) {
                    val entry = archive.getEntry(name) ?: continue
                    if (entry.size > MAX_ENTRY_BYTES) continue
                    bytes = readCapped(archive.getInputStream(entry), MAX_ENTRY_BYTES)
                    break
                }
                bytes
            }
        } catch (err: IOException) {
            // A half-written or corrupt package is worth throwing away: the
            // next run downloads it again instead of failing forever.
            zip.delete()
            throw err
        }
        return found
    }

    private fun studioVersion(): String {
        val cached = versionFile.takeIf { it.isFile }?.readText()?.trim().orEmpty()
        return try {
            val version = connect("https://setup.rbxcdn.com/versionQTStudio", follow = true)
                .use { connection ->
                    if (connection.responseCode !in 200..299) throw IOException("no version")
                    val body = connection.inputStream.use { it.readBytes() }.decodeToString().trim()
                    if (!body.startsWith("version-")) throw IOException("odd version: $body")
                    body
                }
            root.mkdirs()
            versionFile.writeText(version)
            version
        } catch (err: IOException) {
            // Offline, or the CDN is having a moment: an older version's
            // packages are still the right files, as on the desktop.
            if (cached.isEmpty()) throw err
            cached
        }
    }

    // ---- plumbing --------------------------------------------------------

    /** An `HttpURLConnection` that [close]s instead of needing a `finally`. */
    private class Connection(private val inner: HttpURLConnection) : AutoCloseable {
        val responseCode: Int get() = inner.responseCode
        fun getHeaderField(name: String): String? = inner.getHeaderField(name)
        val inputStream: InputStream get() = inner.inputStream
        override fun close() = inner.disconnect()
    }

    private fun connect(url: String, follow: Boolean): Connection {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.instanceFollowRedirects = follow
        connection.connectTimeout = 20_000
        connection.readTimeout = 60_000
        connection.setRequestProperty("User-Agent", USER_AGENT)
        // `Content-Length` has to describe what actually travels: the bytes
        // are handed to the page as is, so no transparent gzip.
        connection.setRequestProperty("Accept-Encoding", "identity")
        return Connection(connection)
    }

    private fun store(file: File, bytes: ByteArray) {
        file.parentFile?.mkdirs()
        val temp = File(file.parentFile, "${file.name}.tmp-${Thread.currentThread().id}")
        temp.writeBytes(bytes)
        if (!temp.renameTo(file)) {
            temp.delete()
        }
    }

    companion object {
        private const val TAG = "rbxview"
        private const val USER_AGENT = "rbx-native/android (+https://github.com/SMSLIMEES/rbx-native)"
        private const val MAX_ASSET_BYTES = 64L * 1024 * 1024
        private const val MAX_PACKAGE_BYTES = 200L * 1024 * 1024
        private const val MAX_ENTRY_BYTES = 64L * 1024 * 1024

        /** See `SUBSTITUTES` in `crates/rbx_assets/src/resolver.rs`. */
        private val substitutes = mapOf(
            "textures/spawnlocation.png" to 6891610111L,
            "textures/ui/guiimageplaceholder.png" to 6293578665L,
        )

        /** See `package_candidates_for_path` in `rbx_assets/src/native.rs`. */
        private fun candidatesFor(path: String): List<String>? =
            when (path.substringBefore('/').lowercase()) {
                "textures" -> listOf("content-textures2.zip", "content-textures3.zip")
                "sky" -> listOf("content-sky.zip", "content-textures3.zip")
                "fonts" -> listOf("content-fonts.zip")
                "sounds" -> listOf("content-sounds.zip")
                "models" -> listOf("content-models.zip")
                "avatar" -> listOf("content-avatar.zip")
                else -> null
            }

        private fun readCapped(stream: InputStream, limit: Long): ByteArray =
            stream.use { input ->
                val buffer = java.io.ByteArrayOutputStream()
                val chunk = ByteArray(64 * 1024)
                var total = 0L
                while (true) {
                    val read = input.read(chunk)
                    if (read < 0) break
                    total += read
                    if (total > limit) throw IOException("response past ${limit / 1024 / 1024} MiB")
                    buffer.write(chunk, 0, read)
                }
                buffer.toByteArray()
            }

        private fun copyCapped(input: InputStream, output: java.io.OutputStream, limit: Long): Long {
            val chunk = ByteArray(256 * 1024)
            var total = 0L
            while (true) {
                val read = input.read(chunk)
                if (read < 0) break
                total += read
                if (total > limit) throw IOException("package past ${limit / 1024 / 1024} MiB")
                output.write(chunk, 0, read)
            }
            return total
        }

        /** Percent-decoding, as `serve.rs::decode`: a malformed escape stays put. */
        fun percentDecode(text: String): String {
            val bytes = text.toByteArray(Charsets.UTF_8)
            val out = java.io.ByteArrayOutputStream(bytes.size)
            var i = 0
            while (i < bytes.size) {
                val byte = bytes[i]
                val hex = if (byte == '%'.code.toByte() && i + 2 < bytes.size) {
                    String(bytes, i + 1, 2, Charsets.US_ASCII).toIntOrNull(16)
                } else null
                if (hex != null) {
                    out.write(hex)
                    i += 3
                } else {
                    out.write(byte.toInt())
                    i += 1
                }
            }
            return out.toByteArray().toString(Charsets.UTF_8)
        }
    }
}
