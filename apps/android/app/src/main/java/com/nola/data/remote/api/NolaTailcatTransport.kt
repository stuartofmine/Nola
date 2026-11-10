package com.nola.data.remote.api

import com.nola.data.local.datastore.PreferenceDataStore
import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.plugins.HttpSend
import io.ktor.client.plugins.plugin
import io.ktor.client.request.HttpRequestBuilder
import io.ktor.client.request.get
import java.io.IOException
import java.net.URI
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.combine
import kotlinx.serialization.json.*

// Native exceptions can contain credential-bearing addresses. Expose only
// fixed stage codes and known SDK errors, never arbitrary exception messages.
internal fun tailcatStartupErrorCode(error: Exception, stage: String): String {
    if (error is java.lang.NoSuchMethodException) return "sdk_binding"
    val native = (error as? java.lang.reflect.InvocationTargetException)?.targetException
    when (native?.message) {
        "encrypted Nola Host inspection failed" -> return "native_host_inspection"
        "encrypted Nola Host inspection failed (dns)" -> return "native_host_dns"
        "encrypted Nola Host inspection failed (certificate)" -> return "native_host_certificate"
        "encrypted Nola Host inspection failed (permission)" -> return "native_host_permission"
        "encrypted Nola Host inspection failed (timeout)" -> return "native_host_timeout"
        "encrypted Nola Host inspection failed (network)" -> return "native_host_network"
        "remote Host inspection refused" -> return "native_host_refused"
        "local transport endpoint unavailable" -> return "native_listener"
        "invalid or non-PSK Tailcat address" -> return "invalid_grant_address"
        "transport startup cancelled" -> return "native_cancelled"
    }
    val phase = stage.takeIf { it in setOf("metadata", "grant", "native_connect", "local_endpoint") } ?: "startup"
    return "$phase:${error.javaClass.simpleName}"
}

/** Optional embedded SDK, memory-only grants. Never changes saved Host identity,
 * credentials, TLS policy, Core permissions, or Run cancellation state. */
@Singleton
class NolaTailcatTransport @Inject constructor(
    private val direct: HttpClient,
    private val preferences: PreferenceDataStore,
    @ApplicationContext context: Context
) {
    @Volatile private var foreground = true
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private class Entry(val configuration: Configuration, val native: Any) {
        @Volatile var startupStage = "metadata"
        lateinit var ready: Deferred<Pair<String, Long>>
        @Volatile var lease: String? = null
    }
    private data class Configuration(val origin: String, val token: String, val host: String)
    private val entries = ConcurrentHashMap<String, Entry>()
    private val guard = Any()
    private val networkInterfaces = TailcatNetworkInterfaces(context.applicationContext)
    private var networkSnapshot: String? = null
    private val nativeClass by lazy { runCatching { Class.forName("nolatailcat.Nolatailcat", false, javaClass.classLoader) }.getOrNull() }
    val available: Boolean get() = nativeClass != null && (javaClass.classLoader as? dalvik.system.BaseDexClassLoader)?.findLibrary("gojni") != null

    init {
        scope.launch {
            combine(preferences.webHostBaseUrl, preferences.webHostToken, preferences.webHostIdentity, preferences.webHostCertificateSha256) { base, token, host, pin ->
                listOf(base, token, host, pin)
            }.collect { values ->
                val current = runCatching { URI(normalizeWebHostBaseUrl(values[0])).let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() } }.getOrNull()
                val obsolete = synchronized(guard) {
                    entries.values.filter { it.configuration.origin != current || it.configuration.token != values[1] || it.configuration.host != values[2] || values[3].isNotEmpty() }
                        .also { previous -> previous.forEach { entries.remove(it.configuration.origin, it) } }
                }
                for (entry in obsolete) { entry.ready.cancel(); close(entry) }
            }
        }
        Handler(Looper.getMainLooper()).post {
            val lifecycle = ProcessLifecycleOwner.get().lifecycle
            if (!lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) park()
            if (foreground && available) scope.launch { runCatching { refreshNetworkInterfaces(observe = true) } }
            lifecycle.addObserver(object : DefaultLifecycleObserver {
                override fun onStart(owner: LifecycleOwner) {
                    foreground = true
                    if (available) scope.launch { runCatching { refreshNetworkInterfaces(observe = true) } }
                }
                override fun onStop(owner: LifecycleOwner) { park() }
            })
        }
    }

    private suspend fun refreshNetworkInterfaces(observe: Boolean) {
        val obsolete = mutableListOf<Entry>()
        try {
            synchronized(guard) {
                if (!foreground) throw IOException("Tailcat transport is suspended until foreground")
                if (observe) networkInterfaces.start {
                    scope.launch { runCatching { refreshNetworkInterfaces(observe = false) } }
                }
                val snapshot = networkInterfaces.snapshot()
                nativeClass!!.getMethod("setAndroidNetworkInterfaces", String::class.java).invoke(null, snapshot)
                if (networkSnapshot != null && networkSnapshot != snapshot) {
                    obsolete.addAll(entries.values)
                    entries.clear()
                }
                networkSnapshot = snapshot
            }
        } catch (failure: Exception) {
            synchronized(guard) {
                networkSnapshot = null
                obsolete.addAll(entries.values)
                entries.clear()
                // Invalid input clears the native snapshot; it never substitutes
                // an empty list as a successful platform enumeration.
                runCatching { nativeClass?.getMethod("setAndroidNetworkInterfaces", String::class.java)?.invoke(null, "") }
            }
            if (failure is CancellationException) throw failure
            throw IOException("Android Tailcat network interfaces unavailable")
        } finally {
            for (entry in obsolete) { entry.ready.cancel(); close(entry) }
        }
    }

    private fun park() {
        // Serialize callback registration with suspension so an in-flight
        // refresh cannot register a new observer after the app went background.
        val pending = synchronized(guard) {
            foreground = false
            networkInterfaces.stop()
            // Keep established memory-only grants; foreground rechecks them.
            entries.values.filter { !it.ready.isCompleted }.also { values -> values.forEach { entries.remove(it.configuration.origin, it) } }
        }
        for (entry in pending) scope.launch { entry.ready.cancel(); close(entry) }
    }

    fun route(client: HttpClient): HttpClient {
        client.plugin(HttpSend).intercept { request ->
            val endpoint = withContext(Dispatchers.IO) { endpoint(request) }
            if (endpoint != null) {
                val target = URI(endpoint)
                request.url.protocol = io.ktor.http.URLProtocol.HTTP
                request.url.host = target.host
                request.url.port = target.port
            }
            execute(request)
        }
        return client
    }

    private suspend fun endpoint(request: HttpRequestBuilder): String? {
        if (!available || request.url.protocol.name != "http" || !URI(request.url.buildString()).rawPath.startsWith("/api/mobile/")) return null
        val base = normalizeWebHostBaseUrl(preferences.webHostBaseUrl.first())
        val origin = runCatching { URI(base).let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() } }.getOrNull() ?: return null
        if (!URI(base).path.isNullOrEmpty() && URI(base).path != "/") return null
        val actual = URI(request.url.buildString())
        if (actual.scheme != "http" || actual.host != URI(origin).host || actual.port != URI(origin).port || preferences.webHostCertificateSha256.first().isNotEmpty()) return null
        val token = preferences.webHostToken.first()
        val host = preferences.webHostIdentity.first()
        if (token.isEmpty() || host.isEmpty()) return null
        val timestamp = request.headers["X-Nola-Mobile-Timestamp"]?.toLongOrNull() ?: return null
        val nonce = request.headers["X-Nola-Mobile-Nonce"] ?: return null
        val digest = request.headers["X-Nola-Mobile-Body-SHA256"] ?: return null
        val expected = mobileAuthProof(token, request.method.value, request.url.buildString(), timestamp, nonce, digest)
        if (!MessageDigest.isEqual(expected.toByteArray(), request.headers["X-Nola-Mobile-Auth"].orEmpty().toByteArray())) return null
        if (!foreground) throw IOException("Tailcat transport is suspended until foreground")
        refreshNetworkInterfaces(observe = true)
        val configuration = Configuration(origin, token, host)
        var previous: Entry? = null
        val entry = synchronized(guard) {
            entries[origin]?.takeIf { it.configuration == configuration && !it.ready.isCancelled } ?: run {
                previous = entries.remove(origin)
                val native = runCatching { requireNotNull(nativeClass!!.getMethod("newSession").invoke(null)) }
                    .getOrElse { throw IOException("Tailcat SDK unavailable") }
                Entry(configuration, native).also { created ->
                    created.ready = scope.async(start = CoroutineStart.LAZY) { connect(created) }
                    entries[origin] = created
                }
            }
        }
        previous?.let { close(it) }
        val ready = try { entry.ready.await() } catch (failure: Exception) {
            if (failure is CancellationException) throw failure
            synchronized(guard) { entries.remove(origin, entry) }
            close(entry)
            throw IOException("Tailcat connection failed (${tailcatStartupErrorCode(failure, entry.startupStage)})")
        }
        if (entries[origin] !== entry) throw IOException("Host transport configuration changed")
        if (ready.second <= System.currentTimeMillis() + 30_000) {
            synchronized(guard) { entries.remove(origin, entry) }; close(entry)
            throw IOException("Tailcat connection expired; retry to reconnect")
        }
        return ready.first.takeIf { it != configuration.origin }
    }

    private suspend fun connect(entry: Entry): Pair<String, Long> {
        val configuration = entry.configuration
        val status = withTimeout(8_000) { direct.get(configuration.origin + "/api/web/session").body<String>() }.let { Json.parseToJsonElement(it).jsonObject }
        require(status["hostId"]?.jsonPrimitive?.content == configuration.host) { "Host identity changed" }
        val advertised = status["tailcatAvailable"]?.jsonPrimitive?.booleanOrNull ?: error("Missing transport capability")
        if (!advertised) return configuration.origin to Long.MAX_VALUE
        entry.startupStage = "grant"
        val public = entry.native.javaClass.getMethod("publicKey").invoke(entry.native) as String
        val requestedAt = System.currentTimeMillis()
        val grant = postEncryptedWebHostRequest(direct, configuration.origin + "/api/mobile/transport/tailcat", configuration.token, buildJsonObject {
            put("operation", "connect"); put("expectedHostId", configuration.host); put("clientPublicKey", public)
        }, maximumResponseBytes = 65536)
        require(grant["hostId"]?.jsonPrimitive?.content == configuration.host)
        entry.lease = grant["leaseId"]?.jsonPrimitive?.content ?: error("Missing transport lease")
        val address = grant["address"]?.jsonPrimitive?.content ?: error("Missing transport grant")
        val seconds = grant["expiresIn"]?.jsonPrimitive?.long ?: error("Missing transport expiry")
        require(seconds in 31..1200)
        entry.startupStage = "native_connect"
        val local = withContext(Dispatchers.IO) {
            entry.native.javaClass.getMethod("connect", String::class.java, String::class.java, String::class.java, Long::class.javaPrimitiveType)
                .invoke(entry.native, address, configuration.host, "", 30_000L) as String
        }
        entry.startupStage = "local_endpoint"
        val uri = URI(local); require(uri.scheme == "http" && uri.host == "127.0.0.1" && uri.port > 0)
        currentCoroutineContext().ensureActive()
        return local to requestedAt + seconds * 1000
    }
    private suspend fun close(entry: Entry) {
        withContext(NonCancellable + Dispatchers.IO) {
            runCatching { entry.native.javaClass.getMethod("close").invoke(entry.native) }
            entry.lease?.also { entry.lease = null }?.let { lease ->
                // Best effort release; the Host's finite lease remains the
                // bound if background suspension interrupts this exchange.
                runCatching { withTimeout(6_000) {
                    postEncryptedWebHostRequest(direct, entry.configuration.origin + "/api/mobile/transport/tailcat", entry.configuration.token,
                        buildJsonObject { put("operation", "close"); put("leaseId", lease) }, maximumResponseBytes = 65536)
                } }
            }
        }
    }
}
