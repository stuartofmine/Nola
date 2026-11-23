package com.nola.data.remote.api

import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.request.HttpRequestBuilder
import io.ktor.client.request.header
import io.ktor.client.request.url
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import io.ktor.http.content.OutgoingContent
import io.ktor.http.headersOf
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NolaTailcatTransportTest {
    private val origin = "http://192.168.1.8:5001"
    private val token = "fixture-shared-token-1234"
    private val hostId = "host-studio-fixture-01"

    private class FakeNetworkMonitor(initialSnapshot: String = """[{"name":"wlan0","up":true}]""") : TailcatNetworkMonitor {
        @Volatile var currentSnapshot: String = initialSnapshot
        @Volatile var onChanged: (() -> Unit)? = null
        val snapshotCalls = AtomicInteger(0)
        val startCalls = AtomicInteger(0)
        val stopCalls = AtomicInteger(0)

        override fun start(changed: () -> Unit) {
            startCalls.incrementAndGet()
            onChanged = changed
        }

        override fun stop() {
            stopCalls.incrementAndGet()
            onChanged = null
        }

        override fun snapshot(): String {
            snapshotCalls.incrementAndGet()
            return currentSnapshot
        }

        fun fireChange(newSnapshot: String) {
            currentSnapshot = newSnapshot
            onChanged?.invoke()
        }
    }

    private class FakeNativeSession(
        val id: Int,
        private val connectHandler: (String, String) -> String,
        private val onClose: () -> Unit = {}
    ) : TailcatNativeSession {
        val closeCalls = AtomicInteger(0)
        override fun publicKey(): String = "pub-key-$id"
        override fun connect(address: String, host: String, extra: String, timeoutMs: Long): String =
            connectHandler(address, host)
        override fun close() {
            closeCalls.incrementAndGet()
            onClose()
        }
    }

    private class FakeNativeRuntime(
        private val onClose: () -> Unit = {},
        private val connectHandler: (Int, String, String) -> String = { id, _, _ -> "http://127.0.0.1:${41000 + id}" }
    ) : TailcatNativeRuntime {
        override val available: Boolean = true
        val interfaceUpdates = CopyOnWriteArrayList<String>()
        val sessions = CopyOnWriteArrayList<FakeNativeSession>()
        private val nextId = AtomicInteger(0)

        override fun setAndroidNetworkInterfaces(snapshot: String) {
            interfaceUpdates.add(snapshot)
        }

        override fun newSession(): TailcatNativeSession {
            val id = nextId.incrementAndGet()
            return FakeNativeSession(id, { address, host -> connectHandler(id, address, host) }, onClose)
                .also { sessions.add(it) }
        }
    }

    private fun signedMobileRequest(path: String = "/api/mobile/agent/settings", requestToken: String = token): HttpRequestBuilder {
        val fullUrl = origin + path
        return HttpRequestBuilder().apply {
            method = HttpMethod.Get
            url(fullUrl)
            mobileAuthHeaders(requestToken, "GET", fullUrl, timestamp = 1_700_000_000L, nonce = "nonce-123456").forEach { (k, v) ->
                header(k, v)
            }
        }
    }

    private fun encryptedResponse(payload: JsonObject): String =
        Json { encodeDefaults = true }.encodeToString(MobileCrypto.encryptJsonObject(payload, token))

    private fun decryptRequestBody(body: OutgoingContent): JsonObject {
        val bytes = (body as OutgoingContent.ByteArrayContent).bytes()
        val envelope = Json.decodeFromString<EncryptedEnvelope>(bytes.decodeToString())
        return MobileCrypto.decryptJsonObject(envelope, token)
    }

    @Test
    fun expiredLeaseAutoReconnectsInsteadOfThrowing() = runBlocking {
        val clock = AtomicLong(1_000_000L)
        val operations = CopyOnWriteArrayList<String>()
        val leaseSeq = AtomicInteger(0)
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> {
                            val req = decryptRequestBody(request.body)
                            val op = req["operation"]!!.jsonPrimitive.content
                            operations.add(op)
                            when (op) {
                                "connect" -> {
                                    val lease = "lease-${leaseSeq.incrementAndGet()}"
                                    respond(
                                        content = encryptedResponse(buildJsonObject {
                                            put("hostId", hostId)
                                            put("leaseId", lease)
                                            put("address", "psk://grant-$lease")
                                            put("expiresIn", 60L)
                                        }),
                                        status = HttpStatusCode.OK,
                                        headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                    )
                                }
                                "close" -> respond(
                                    content = encryptedResponse(buildJsonObject { put("closed", true) }),
                                    status = HttpStatusCode.OK,
                                    headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                )
                                else -> respond("", HttpStatusCode.BadRequest)
                            }
                        }
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val monitor = FakeNetworkMonitor()
        val runtime = FakeNativeRuntime()
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = monitor,
            nativeRuntime = runtime,
            nowMs = { clock.get() },
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        val first = transport.endpoint(signedMobileRequest())
        assertEquals("http://127.0.0.1:41001", first)
        assertEquals(1, runtime.sessions.size)

        // Advance into the 30s expiry window (60s lease, 35s elapsed -> 25s left).
        clock.addAndGet(35_000L)
        val second = transport.endpoint(signedMobileRequest())
        assertEquals("http://127.0.0.1:41002", second)
        assertEquals(2, runtime.sessions.size)
        assertEquals(1, runtime.sessions[0].closeCalls.get())
        assertTrue(operations.containsAll(listOf("connect", "close")))
        assertEquals(2, operations.count { it == "connect" })
        client.close()
    }

    @Test
    fun proactiveRenewExtendsActiveLeaseWithoutReconnecting() = runBlocking {
        val clock = AtomicLong(1_000_000L)
        val renewDone = CompletableDeferred<Unit>()
        val operations = CopyOnWriteArrayList<String>()
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> {
                            val req = decryptRequestBody(request.body)
                            val op = req["operation"]!!.jsonPrimitive.content
                            operations.add(op)
                            when (op) {
                                "connect" -> respond(
                                    content = encryptedResponse(buildJsonObject {
                                        put("hostId", hostId)
                                        put("leaseId", "lease-1")
                                        put("address", "psk://grant-1")
                                        put("expiresIn", 600L)
                                    }),
                                    status = HttpStatusCode.OK,
                                    headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                )
                                "renew" -> {
                                    assertEquals("lease-1", req["leaseId"]?.jsonPrimitive?.content)
                                    renewDone.complete(Unit)
                                    respond(
                                        content = encryptedResponse(buildJsonObject {
                                            put("hostId", hostId)
                                            put("leaseId", "lease-1")
                                            put("expiresIn", 600L)
                                        }),
                                        status = HttpStatusCode.OK,
                                        headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                    )
                                }
                                else -> respond("", HttpStatusCode.BadRequest)
                            }
                        }
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = FakeNetworkMonitor(),
            nativeRuntime = FakeNativeRuntime(),
            nowMs = { clock.get() },
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        val first = transport.endpoint(signedMobileRequest())
        assertEquals("http://127.0.0.1:41001", first)

        // Advance to 460s elapsed (140s remaining <= 25% of 600s = 150s): triggers non-blocking renew.
        clock.addAndGet(460_000L)
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        withTimeout(5_000) { renewDone.await() }
        // Give the renew coroutine a moment to update entry.expiresAtMs.
        delay(50)

        // Advance past original 600s expiry (now at 650s elapsed); renewed lease is valid until 460+600=1060s!
        clock.addAndGet(190_000L)
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        assertEquals(1, operations.count { it == "connect" })
        assertEquals(1, operations.count { it == "renew" })
        client.close()
    }

    @Test
    fun renewUnsupportedFallsBackCleanlyToReconnectAtExpiry() = runBlocking {
        val clock = AtomicLong(1_000_000L)
        val renewAttempted = CompletableDeferred<Unit>()
        val operations = CopyOnWriteArrayList<String>()
        val leaseSeq = AtomicInteger(0)
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> {
                            val req = decryptRequestBody(request.body)
                            val op = req["operation"]!!.jsonPrimitive.content
                            operations.add(op)
                            when (op) {
                                "connect" -> {
                                    val lease = "lease-${leaseSeq.incrementAndGet()}"
                                    respond(
                                        content = encryptedResponse(buildJsonObject {
                                            put("hostId", hostId)
                                            put("leaseId", lease)
                                            put("address", "psk://grant-$lease")
                                            put("expiresIn", 200L)
                                        }),
                                        status = HttpStatusCode.OK,
                                        headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                    )
                                }
                                "renew" -> {
                                    renewAttempted.complete(Unit)
                                    respond(
                                        content = encryptedResponse(buildJsonObject {
                                            put("code", "unknown_transport_operation")
                                        }),
                                        status = HttpStatusCode.BadRequest,
                                        headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                    )
                                }
                                "close" -> respond(
                                    content = encryptedResponse(buildJsonObject { put("closed", true) }),
                                    status = HttpStatusCode.OK,
                                    headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                )
                                else -> respond("", HttpStatusCode.BadRequest)
                            }
                        }
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = FakeNetworkMonitor(),
            nativeRuntime = FakeNativeRuntime(),
            nowMs = { clock.get() },
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        // Trigger renew (45s remaining <= 50s renew window).
        clock.addAndGet(155_000L)
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        withTimeout(5_000) { renewAttempted.await() }
        delay(50)

        // Subsequent request before 30s expiry does NOT retry unsupported renew.
        clock.addAndGet(10_000L)
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        delay(50)
        assertEquals(1, operations.count { it == "renew" })

        // Enter 30s expiry window -> cleanly reconnects via "connect".
        clock.addAndGet(10_000L)
        assertEquals("http://127.0.0.1:41002", transport.endpoint(signedMobileRequest()))
        assertEquals(2, operations.count { it == "connect" })
        client.close()
    }

    @Test
    fun parkThrowsTailcatConnectionExceptionInsteadOfCancellingActiveCaller() = runBlocking {
        val connectEntered = CompletableDeferred<Unit>()
        val releaseConnect = CompletableDeferred<Unit>()
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> {
                            val req = decryptRequestBody(request.body)
                            when (req["operation"]!!.jsonPrimitive.content) {
                                "connect" -> respond(
                                    content = encryptedResponse(buildJsonObject {
                                        put("hostId", hostId)
                                        put("leaseId", "lease-park")
                                        put("address", "psk://grant-park")
                                        put("expiresIn", 600L)
                                    }),
                                    status = HttpStatusCode.OK,
                                    headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                )
                                else -> respond(
                                    content = encryptedResponse(buildJsonObject { put("closed", true) }),
                                    status = HttpStatusCode.OK,
                                    headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                                )
                            }
                        }
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val runtime = FakeNativeRuntime(onClose = { releaseConnect.complete(Unit) }) { _, _, _ ->
            connectEntered.complete(Unit)
            runBlocking { releaseConnect.await() }
            "http://127.0.0.1:41099"
        }
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = FakeNetworkMonitor(),
            nativeRuntime = runtime,
            nowMs = System::currentTimeMillis,
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        val caller = async(Dispatchers.IO) {
            runCatching { transport.endpoint(signedMobileRequest()) }
        }
        withTimeout(5_000) { connectEntered.await() }
        transport.park()
        val result = withTimeout(5_000) { caller.await() }
        releaseConnect.complete(Unit)

        val failure = result.exceptionOrNull()
        assertTrue("Expected TailcatConnectionException but got $failure", failure is TailcatConnectionException)
        assertEquals("transport_suspended", (failure as TailcatConnectionException).code)
        client.close()
    }

    @Test
    fun networkInterfaceChangesPreserveActiveTunnelsAndAvoidPerRequestEnumeration() = runBlocking {
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> respond(
                            content = encryptedResponse(buildJsonObject {
                                put("hostId", hostId)
                                put("leaseId", "lease-roam")
                                put("address", "psk://grant-roam")
                                put("expiresIn", 600L)
                            }),
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val monitor = FakeNetworkMonitor("""[{"name":"wlan0","up":true}]""")
        val runtime = FakeNativeRuntime()
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = monitor,
            nativeRuntime = runtime,
            nowMs = System::currentTimeMillis,
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        // First request initializes snapshot once.
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        assertEquals(1, monitor.snapshotCalls.get())
        assertEquals(1, runtime.interfaceUpdates.size)

        // Subsequent requests do NOT re-enumerate network interfaces synchronously.
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        assertEquals(1, monitor.snapshotCalls.get())

        // Roam from Wi-Fi to 5G: updates native snapshot WITHOUT closing or wiping active tunnel!
        val cellularSnapshot = """[{"name":"rmnet0","up":true}]"""
        monitor.fireChange(cellularSnapshot)
        withTimeout(5_000) {
            while (runtime.interfaceUpdates.size < 2) delay(10)
        }
        assertEquals(cellularSnapshot, runtime.interfaceUpdates.last())
        assertEquals(0, runtime.sessions[0].closeCalls.get())
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        assertEquals(1, runtime.sessions.size)

        // Transient empty snapshot mid-roam must NOT clear native snapshot or close active tunnel!
        monitor.fireChange("[]")
        delay(50)
        assertEquals(cellularSnapshot, runtime.interfaceUpdates.last())
        assertEquals(0, runtime.sessions[0].closeCalls.get())
        assertEquals("http://127.0.0.1:41001", transport.endpoint(signedMobileRequest()))
        client.close()
    }

    @Test
    fun draftPairingExercisesTailcatBeforePersistence() = runBlocking {
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> respond(
                            content = encryptedResponse(buildJsonObject {
                                put("hostId", hostId)
                                put("leaseId", "lease-draft")
                                put("address", "psk://grant-draft")
                                put("expiresIn", 600L)
                            }),
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings("", "", "", "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = FakeNetworkMonitor(),
            nativeRuntime = FakeNativeRuntime(),
            nowMs = System::currentTimeMillis,
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        // Before saving and outside draft pairing, Tailcat does not route unconfigured origins.
        assertNull(transport.endpoint(signedMobileRequest()))

        // Inside withDraftPairing, initial pairing exercises Tailcat.
        val routed = transport.withDraftPairing(origin, token, hostId) {
            transport.endpoint(signedMobileRequest())
        }
        assertNotNull(routed)
        assertEquals("http://127.0.0.1:41001", routed)
        client.close()
    }

    @Test
    fun grantErrorCodePropagatesToTailcatConnectionException() = runBlocking {
        val client = HttpClient(MockEngine) {
            engine {
                addHandler { request ->
                    when (request.url.encodedPath) {
                        "/api/web/session" -> respond(
                            content = """{"hostId":"$hostId","tailcatAvailable":true}""",
                            status = HttpStatusCode.OK,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        "/api/mobile/transport/tailcat" -> respond(
                            content = encryptedResponse(buildJsonObject {
                                put("code", "transport_capacity_reached")
                            }),
                            status = HttpStatusCode.TooManyRequests,
                            headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString())
                        )
                        else -> respond("", HttpStatusCode.NotFound)
                    }
                }
            }
        }
        val transport = NolaTailcatTransport(
            direct = client,
            readSettings = { TailcatHostSettings(origin, token, hostId, "") },
            settingsChanges = emptyFlow(),
            networkInterfaces = FakeNetworkMonitor(),
            nativeRuntime = FakeNativeRuntime(),
            nowMs = System::currentTimeMillis,
            observeLifecycle = false,
            networkRefreshDebounceMs = 0L,
            initialForeground = true
        )

        val failure = runCatching { transport.endpoint(signedMobileRequest()) }.exceptionOrNull()
        assertTrue(failure is TailcatConnectionException)
        assertEquals("transport_capacity_reached", (failure as TailcatConnectionException).code)
        client.close()
    }
}
