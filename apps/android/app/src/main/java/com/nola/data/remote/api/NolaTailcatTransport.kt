package com.nola.data.remote.api

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.nola.data.local.datastore.PreferenceDataStore
import dagger.hilt.android.qualifiers.ApplicationContext
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.plugins.HttpSend
import io.ktor.client.plugins.plugin
import io.ktor.client.request.HttpRequestBuilder
import io.ktor.client.request.get
import java.io.IOException
import java.net.ConnectException
import java.net.URI
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import kotlinx.serialization.json.*

// Native exceptions can contain credential-bearing addresses. Expose only
// fixed stage codes and known SDK errors, never arbitrary exception messages.
internal fun tailcatStartupErrorCode(error: Exception, stage: String): String {
    if (error is java.lang.NoSuchMethodException) return "sdk_binding"
    if (error is WebHostStatusException) {
        val code = error.errorCode
        if (!code.isNullOrBlank()) return code
    }
    if (stage == "metadata" && error.message == "Host identity changed") {
        return "transport_host_identity_mismatch"
    }
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

internal val tailcatRoutedRequest = io.ktor.util.AttributeKey<Boolean>("NolaTailcatRoutedRequest")

internal class TailcatConnectionException(
    val code: String,
    detail: String = "Tailcat connection failed ($code)"
) : IOException(detail)

internal interface TailcatNativeSession {
    fun publicKey(): String
    fun connect(address: String, host: String, extra: String, timeoutMs: Long): String
    fun close()
}

internal interface TailcatNativeRuntime {
    val available: Boolean
    fun setAndroidNetworkInterfaces(snapshot: String)
    fun newSession(): TailcatNativeSession
}

internal data class TailcatHostSettings(
    val baseUrl: String,
    val token: String,
    val hostId: String,
    val certificateSha256: String
)

private class ReflectiveTailcatNativeRuntime(private val classLoader: ClassLoader?) : TailcatNativeRuntime {
    private val nativeClass by lazy {
        runCatching { Class.forName("nolatailcat.Nolatailcat", false, classLoader) }.getOrNull()
    }
    override val available: Boolean
        get() = nativeClass != null && (classLoader as? dalvik.system.BaseDexClassLoader)?.findLibrary("gojni") != null

    override fun setAndroidNetworkInterfaces(snapshot: String) {
        nativeClass!!.getMethod("setAndroidNetworkInterfaces", String::class.java).invoke(null, snapshot)
    }

    override fun newSession(): TailcatNativeSession {
        val raw = runCatching { requireNotNull(nativeClass!!.getMethod("newSession").invoke(null)) }
            .getOrElse { throw IOException("Tailcat SDK unavailable") }
        return object : TailcatNativeSession {
            override fun publicKey(): String = raw.javaClass.getMethod("publicKey").invoke(raw) as String
            override fun connect(address: String, host: String, extra: String, timeoutMs: Long): String =
                raw.javaClass.getMethod(
                    "connect",
                    String::class.java,
                    String::class.java,
                    String::class.java,
                    Long::class.javaPrimitiveType
                ).invoke(raw, address, host, extra, timeoutMs) as String

            override fun close() {
                raw.javaClass.getMethod("close").invoke(raw)
            }
        }
    }
}

/** Optional embedded SDK, memory-only grants. Never changes saved Host identity,
 * credentials, TLS policy, Core permissions, or Run cancellation state. */
@Singleton
class NolaTailcatTransport internal constructor(
    private val direct: HttpClient,
    private val readSettings: suspend () -> TailcatHostSettings,
    private val settingsChanges: Flow<TailcatHostSettings>,
    private val networkInterfaces: TailcatNetworkMonitor,
    private val nativeRuntime: TailcatNativeRuntime,
    private val nowMs: () -> Long,
    private val observeLifecycle: Boolean,
    private val networkRefreshDebounceMs: Long,
    initialForeground: Boolean
) {
    @Inject
    constructor(
        direct: HttpClient,
        preferences: PreferenceDataStore,
        @ApplicationContext context: Context
    ) : this(
        direct = direct,
        readSettings = {
            TailcatHostSettings(
                baseUrl = preferences.webHostBaseUrl.first(),
                token = preferences.webHostToken.first(),
                hostId = preferences.webHostIdentity.first(),
                certificateSha256 = preferences.webHostCertificateSha256.first()
            )
        },
        settingsChanges = combine(
            preferences.webHostBaseUrl,
            preferences.webHostToken,
            preferences.webHostIdentity,
            preferences.webHostCertificateSha256
        ) { base, token, host, pin -> TailcatHostSettings(base, token, host, pin) },
        networkInterfaces = TailcatNetworkInterfaces(context.applicationContext),
        nativeRuntime = ReflectiveTailcatNativeRuntime(NolaTailcatTransport::class.java.classLoader),
        nowMs = System::currentTimeMillis,
        observeLifecycle = true,
        networkRefreshDebounceMs = 250L,
        initialForeground = initialProcessForegroundState()
    )

    @Volatile private var foreground = initialForeground
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private data class EndpointLease(val endpoint: String, val expiresAtMs: Long, val durationMs: Long)
    private data class Configuration(val origin: String, val token: String, val host: String)
    private data class TransientPairing(val origin: String, val token: String, val host: String, val pin: String)

    private class Entry(val configuration: Configuration, val native: TailcatNativeSession) {
        @Volatile var startupStage = "metadata"
        lateinit var ready: Deferred<EndpointLease>
        @Volatile var lease: String? = null
        @Volatile var resolvedEndpoint: String? = null
        @Volatile var expiresAtMs: Long = Long.MAX_VALUE
        @Volatile var leaseDurationMs: Long = 0L
        @Volatile var renewUnsupported: Boolean = false
        @Volatile var renewInFlight: Boolean = false
        @Volatile var lastRenewAttemptMs: Long = 0L
    }

    private val entries = ConcurrentHashMap<String, Entry>()
    private val guard = Any()
    @Volatile private var networkSnapshot: String? = null
    @Volatile private var transientPairing: TransientPairing? = null
    private var networkRefreshJob: Job? = null

    val available: Boolean get() = nativeRuntime.available

    init {
        scope.launch {
            settingsChanges.collect { values ->
                val current = runCatching {
                    URI(normalizeWebHostBaseUrl(values.baseUrl)).let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() }
                }.getOrNull()
                val draft = transientPairing
                val obsolete = synchronized(guard) {
                    entries.values.filter { entry ->
                        val matchesDraft = draft != null &&
                            entry.configuration.origin == draft.origin &&
                            entry.configuration.token == draft.token &&
                            (draft.host.isEmpty() || entry.configuration.host == draft.host) &&
                            draft.pin.isEmpty()
                        !matchesDraft && (
                            entry.configuration.origin != current ||
                                entry.configuration.token != values.token ||
                                entry.configuration.host != values.hostId ||
                                values.certificateSha256.isNotEmpty()
                            )
                    }.also { previous -> previous.forEach { entries.remove(it.configuration.origin, it) } }
                }
                for (entry in obsolete) { entry.ready.cancel(); close(entry) }
            }
        }
        if (observeLifecycle) {
            Handler(Looper.getMainLooper()).post {
                val lifecycle = ProcessLifecycleOwner.get().lifecycle
                val started = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
                foreground = started
                if (!started) {
                    park()
                } else if (available) {
                    scope.launch { runCatching { refreshNetworkInterfaces(observe = true, initial = false) } }
                }
                lifecycle.addObserver(object : DefaultLifecycleObserver {
                    override fun onStart(owner: LifecycleOwner) {
                        foreground = true
                        if (available) scope.launch { runCatching { refreshNetworkInterfaces(observe = true, initial = false) } }
                    }
                    override fun onStop(owner: LifecycleOwner) { park() }
                })
            }
        }
    }

    /**
     * Temporarily registers unpersisted pairing credentials so initial pairing
     * verification can exercise and verify Tailcat before saving to DataStore.
     */
    suspend fun <T> withDraftPairing(
        baseUrl: String,
        token: String,
        hostId: String?,
        certificateSha256: String = "",
        block: suspend () -> T
    ): T {
        val normalized = normalizeWebHostBaseUrl(baseUrl)
        val uri = runCatching { URI(normalized) }.getOrNull()
        val origin = uri?.let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() }
        val draft = if (origin != null && (uri.path.isNullOrEmpty() || uri.path == "/") && token.isNotBlank()) {
            TransientPairing(origin, token.trim(), hostId.orEmpty().trim(), certificateSha256.trim())
        } else {
            null
        }
        val previous = transientPairing
        transientPairing = draft
        try {
            return block()
        } finally {
            transientPairing = previous
        }
    }

    private fun scheduleNetworkRefresh() {
        synchronized(guard) {
            if (!foreground) return
            networkRefreshJob?.cancel()
            networkRefreshJob = scope.launch {
                if (networkRefreshDebounceMs > 0) delay(networkRefreshDebounceMs)
                runCatching { refreshNetworkInterfaces(observe = false, initial = false) }
            }
        }
    }

    internal suspend fun refreshNetworkInterfaces(observe: Boolean, initial: Boolean = false) {
        synchronized(guard) {
            if (!foreground) {
                throw TailcatConnectionException("transport_suspended", "Tailcat transport is suspended until foreground")
            }
            if (observe) {
                networkInterfaces.start { scheduleNetworkRefresh() }
            }
        }
        val snapshot = try {
            networkInterfaces.snapshot()
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            if (networkSnapshot == null && initial) {
                throw IOException("Android Tailcat network interfaces unavailable")
            }
            return
        }
        // Ignore transient empty-interface hiccups mid-roam when a prior snapshot exists;
        // never clear the native snapshot or tear down active tunnels during handoff.
        if (snapshot == "[]" && networkSnapshot != null) return
        try {
            synchronized(guard) {
                if (!foreground) {
                    throw TailcatConnectionException("transport_suspended", "Tailcat transport is suspended until foreground")
                }
                if (networkSnapshot != snapshot) {
                    nativeRuntime.setAndroidNetworkInterfaces(snapshot)
                    networkSnapshot = snapshot
                }
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (suspended: TailcatConnectionException) {
            throw suspended
        } catch (_: Exception) {
            if (networkSnapshot == null && initial) {
                throw IOException("Android Tailcat network interfaces unavailable")
            }
        }
    }

    internal fun park() {
        // Serialize callback registration with suspension so an in-flight
        // refresh cannot register a new observer after the app went background.
        val pending = synchronized(guard) {
            foreground = false
            networkRefreshJob?.cancel()
            networkRefreshJob = null
            networkInterfaces.stop()
            // Keep established memory-only grants; foreground rechecks them.
            entries.values.filter { !it.ready.isCompleted }.also { values -> values.forEach { entries.remove(it.configuration.origin, it) } }
        }
        for (entry in pending) {
            entry.ready.cancel()
            scope.launch { close(entry) }
        }
    }

    internal fun resumeForTest() {
        synchronized(guard) { foreground = true }
    }

    fun route(client: HttpClient): HttpClient {
        client.plugin(HttpSend).intercept { request ->
            val originalUri = runCatching { URI(request.url.buildString()) }.getOrNull()
            val originalOrigin = originalUri?.let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() }
            val endpoint = withContext(Dispatchers.IO) { endpoint(request) }
            if (endpoint != null) {
                request.attributes.put(tailcatRoutedRequest, true)
                val target = URI(endpoint)
                request.url.protocol = io.ktor.http.URLProtocol.HTTP
                request.url.host = target.host
                request.url.port = target.port
            }
            try {
                execute(request)
            } catch (connectFailure: ConnectException) {
                if (endpoint != null && originalOrigin != null) {
                    invalidateEndpoint(originalOrigin, endpoint)
                }
                throw connectFailure
            }
        }
        return client
    }

    private fun invalidateEndpoint(origin: String, endpoint: String) {
        val removed = synchronized(guard) {
            val current = entries[origin]
            if (current != null && current.resolvedEndpoint == endpoint) {
                entries.remove(origin, current)
                current
            } else {
                null
            }
        }
        if (removed != null) {
            scope.launch { close(removed) }
        }
    }

    private fun maybeScheduleLeaseRenewal(entry: Entry, now: Long) {
        if (entry.lease == null || entry.renewUnsupported || entry.leaseDurationMs <= 0L) return
        val remainingMs = entry.expiresAtMs - now
        val renewWindowMs = minOf(300_000L, maxOf(45_000L, entry.leaseDurationMs / 4))
        if (remainingMs > renewWindowMs) return
        val shouldStart = synchronized(guard) {
            if (entry.renewInFlight || entry.renewUnsupported || now - entry.lastRenewAttemptMs < 15_000L) {
                false
            } else {
                entry.renewInFlight = true
                entry.lastRenewAttemptMs = now
                true
            }
        }
        if (!shouldStart) return
        scope.launch { renewLease(entry) }
    }

    private suspend fun renewLease(entry: Entry) {
        try {
            val leaseId = entry.lease ?: return
            val requestedAt = nowMs()
            val response = withTimeout(10_000) {
                postEncryptedWebHostRequest(
                    direct,
                    entry.configuration.origin + "/api/mobile/transport/tailcat",
                    entry.configuration.token,
                    buildJsonObject {
                        put("operation", "renew")
                        put("leaseId", leaseId)
                    },
                    maximumResponseBytes = 65536
                )
            }
            val responseHost = response["hostId"]?.jsonPrimitive?.content
            if (responseHost != null && entry.configuration.host.isNotEmpty()) {
                require(responseHost == entry.configuration.host)
            }
            val seconds = response["expiresIn"]?.jsonPrimitive?.long ?: error("Missing transport expiry")
            require(seconds in 31..1200)
            val updatedLease = response["leaseId"]?.jsonPrimitive?.content?.takeIf { it.isNotBlank() }
            synchronized(guard) {
                if (updatedLease != null) entry.lease = updatedLease
                entry.leaseDurationMs = seconds * 1000L
                entry.expiresAtMs = requestedAt + seconds * 1000L
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (status: WebHostStatusException) {
            if ((status.statusCode == 400 && status.errorCode == "unknown_transport_operation") || status.statusCode == 404) {
                entry.renewUnsupported = true
            }
        } catch (_: Exception) {
            // Transient renewal failure; falls back cleanly to reconnect at the 30s expiry window.
        } finally {
            synchronized(guard) { entry.renewInFlight = false }
        }
    }

    internal suspend fun endpoint(request: HttpRequestBuilder): String? {
        if (!available || request.url.protocol.name != "http" || !URI(request.url.buildString()).rawPath.startsWith("/api/mobile/")) return null
        val actual = URI(request.url.buildString())
        val actualOrigin = runCatching { URI(actual.scheme, null, actual.host, actual.port, null, null, null).toString() }.getOrNull() ?: return null

        val draft = transientPairing
        val configuration: Configuration = if (draft != null && draft.origin == actualOrigin && draft.pin.isEmpty()) {
            Configuration(draft.origin, draft.token, draft.host)
        } else {
            val saved = readSettings()
            val base = normalizeWebHostBaseUrl(saved.baseUrl)
            val origin = runCatching { URI(base).let { URI(it.scheme, null, it.host, it.port, null, null, null).toString() } }.getOrNull() ?: return null
            if (!URI(base).path.isNullOrEmpty() && URI(base).path != "/") return null
            if (actual.scheme != "http" || actual.host != URI(origin).host || actual.port != URI(origin).port || saved.certificateSha256.isNotEmpty()) return null
            if (saved.token.isEmpty() || saved.hostId.isEmpty()) return null
            Configuration(origin, saved.token, saved.hostId)
        }

        val timestamp = request.headers["X-Nola-Mobile-Timestamp"]?.toLongOrNull() ?: return null
        val nonce = request.headers["X-Nola-Mobile-Nonce"] ?: return null
        val digest = request.headers["X-Nola-Mobile-Body-SHA256"] ?: return null
        val expected = mobileAuthProof(configuration.token, request.method.value, request.url.buildString(), timestamp, nonce, digest)
        if (!MessageDigest.isEqual(expected.toByteArray(), request.headers["X-Nola-Mobile-Auth"].orEmpty().toByteArray())) return null
        if (!foreground) throw TailcatConnectionException("transport_suspended", "Tailcat transport is suspended until foreground")

        if (networkSnapshot == null) {
            refreshNetworkInterfaces(observe = true, initial = true)
        }

        val origin = configuration.origin
        for (attempt in 0 until 2) {
            var previous: Entry? = null
            val entry = synchronized(guard) {
                entries[origin]?.takeIf { it.configuration == configuration && !it.ready.isCancelled } ?: run {
                    previous = entries.remove(origin)
                    val native = nativeRuntime.newSession()
                    Entry(configuration, native).also { created ->
                        created.ready = scope.async(start = CoroutineStart.LAZY) { connect(created) }
                        entries[origin] = created
                    }
                }
            }
            previous?.let { close(it) }
            val ready = try {
                entry.ready.await()
            } catch (cancelled: CancellationException) {
                currentCoroutineContext().ensureActive()
                throw TailcatConnectionException("transport_suspended", "Tailcat transport is suspended until foreground")
            } catch (failure: Exception) {
                synchronized(guard) { entries.remove(origin, entry) }
                close(entry)
                val code = tailcatStartupErrorCode(failure, entry.startupStage)
                throw TailcatConnectionException(code, "Tailcat connection failed ($code)")
            }
            if (entries[origin] !== entry) throw IOException("Host transport configuration changed")
            val now = nowMs()
            if (entry.expiresAtMs <= now + 30_000L) {
                val removed = synchronized(guard) { entries.remove(origin, entry) }
                if (removed) close(entry)
                if (attempt == 0) continue
                throw TailcatConnectionException("tailcat_lease_expired", "Tailcat connection expired")
            }
            if (ready.endpoint != configuration.origin) {
                maybeScheduleLeaseRenewal(entry, now)
            }
            return ready.endpoint.takeIf { it != configuration.origin }
        }
        throw TailcatConnectionException("tailcat_lease_expired", "Tailcat connection expired")
    }

    private suspend fun connect(entry: Entry): EndpointLease {
        val configuration = entry.configuration
        val status = withTimeout(8_000) { direct.get(configuration.origin + "/api/web/session").body<String>() }.let { Json.parseToJsonElement(it).jsonObject }
        val sessionHostId = status["hostId"]?.jsonPrimitive?.content.orEmpty()
        if (configuration.host.isNotEmpty()) {
            require(sessionHostId == configuration.host) { "Host identity changed" }
        } else {
            require(sessionHostId.isNotEmpty()) { "Missing host identity" }
        }
        val effectiveHostId = configuration.host.ifEmpty { sessionHostId }
        val advertised = status["tailcatAvailable"]?.jsonPrimitive?.booleanOrNull ?: error("Missing transport capability")
        if (!advertised) {
            entry.expiresAtMs = Long.MAX_VALUE
            entry.leaseDurationMs = 0L
            return EndpointLease(configuration.origin, Long.MAX_VALUE, 0L)
        }
        entry.startupStage = "grant"
        val public = entry.native.publicKey()
        val requestedAt = nowMs()
        val grant = postEncryptedWebHostRequest(direct, configuration.origin + "/api/mobile/transport/tailcat", configuration.token, buildJsonObject {
            put("operation", "connect"); put("expectedHostId", effectiveHostId); put("clientPublicKey", public)
        }, maximumResponseBytes = 65536)
        require(grant["hostId"]?.jsonPrimitive?.content == effectiveHostId)
        entry.lease = grant["leaseId"]?.jsonPrimitive?.content ?: error("Missing transport lease")
        val address = grant["address"]?.jsonPrimitive?.content ?: error("Missing transport grant")
        val seconds = grant["expiresIn"]?.jsonPrimitive?.long ?: error("Missing transport expiry")
        require(seconds in 31..1200)
        entry.startupStage = "native_connect"
        val local = withContext(Dispatchers.IO) {
            entry.native.connect(address, effectiveHostId, "", 30_000L)
        }
        entry.startupStage = "local_endpoint"
        val uri = URI(local); require(uri.scheme == "http" && uri.host == "127.0.0.1" && uri.port > 0)
        currentCoroutineContext().ensureActive()
        val durationMs = seconds * 1000L
        val expiresAt = requestedAt + durationMs
        entry.resolvedEndpoint = local
        entry.leaseDurationMs = durationMs
        entry.expiresAtMs = expiresAt
        return EndpointLease(local, expiresAt, durationMs)
    }

    private suspend fun close(entry: Entry) {
        withContext(NonCancellable + Dispatchers.IO) {
            runCatching { entry.native.close() }
            entry.lease?.also { entry.lease = null }?.let { lease ->
                // Best effort release; the Host's finite lease remains the
                // bound if background suspension interrupts this exchange.
                runCatching {
                    withTimeout(6_000) {
                        postEncryptedWebHostRequest(
                            direct,
                            entry.configuration.origin + "/api/mobile/transport/tailcat",
                            entry.configuration.token,
                            buildJsonObject { put("operation", "close"); put("leaseId", lease) },
                            maximumResponseBytes = 65536
                        )
                    }
                }
            }
        }
    }

    companion object {
        private fun initialProcessForegroundState(): Boolean = runCatching {
            if (Looper.myLooper() == Looper.getMainLooper()) {
                ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            } else {
                true
            }
        }.getOrDefault(true)
    }
}
