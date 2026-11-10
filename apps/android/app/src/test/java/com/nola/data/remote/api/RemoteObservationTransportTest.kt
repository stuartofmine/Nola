package com.nola.data.remote.api

import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseStatusException
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.http.HttpStatusCode
import io.ktor.http.content.OutgoingContent
import io.ktor.http.headersOf
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.flow.toList
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.*
import kotlin.test.*

/** Both SSE and the production's independent control engine hit this real server. */
private class EncryptedControlLoopback(
    private val token: String,
    private val respond: (String, JsonObject) -> Pair<String, String>,
) : java.io.Closeable {
    private val socket = java.net.ServerSocket(0, 8, java.net.InetAddress.getByName("127.0.0.1"))
    val baseUrl = "http://127.0.0.1:${socket.localPort}"
    private val failure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
    private val worker = java.util.concurrent.Executors.newSingleThreadExecutor { task ->
        Thread(task, "encrypted-control-loopback").apply { isDaemon = true }
    }

    init {
        worker.submit {
            try {
                while (!socket.isClosed) socket.accept().use { connection ->
                    connection.soTimeout = 5_000
                    val input = connection.getInputStream().buffered()
                    fun line(): String {
                        val bytes = java.io.ByteArrayOutputStream()
                        while (true) {
                            val byte = input.read()
                            check(byte >= 0) { "Truncated HTTP headers" }
                            if (byte == 10) break
                            if (byte != 13) bytes.write(byte)
                        }
                        return bytes.toString(Charsets.ISO_8859_1.name())
                    }
                    val requestLine = line().split(' ')
                    assertEquals("POST", requestLine[0])
                    val path = requestLine[1]
                    val headers = mutableMapOf<String, String>()
                    while (true) {
                        val header = line()
                        if (header.isEmpty()) break
                        headers[header.substringBefore(':').lowercase()] = header.substringAfter(':').trim()
                    }
                    val size = headers.getValue("content-length").toInt()
                    require(size in 1..1_048_576)
                    val bytes = input.readNBytes(size)
                    assertEquals(size, bytes.size)
                    val raw = bytes.decodeToString()
                    assertFalse(raw.contains("原始正文"))
                    assertEquals("aes-gcm-v1", headers["x-nola-mobile-crypto"])
                    assertNull(headers["authorization"])
                    val expectedHeaders = mobileAuthHeaders(token, "POST", baseUrl + path,
                        headers.getValue("x-nola-mobile-timestamp").toLong(),
                        headers.getValue("x-nola-mobile-nonce"), raw)
                    expectedHeaders.forEach { (name, value) -> assertEquals(value, headers[name.lowercase()]) }
                    val clear = MobileCrypto.decryptJsonObject(Json.decodeFromString<EncryptedEnvelope>(raw), token)
                    val (type, payload) = respond(path, clear)
                    val body = payload.toByteArray()
                    val output = connection.getOutputStream()
                    output.write("HTTP/1.1 200 OK\r\nContent-Type: $type\r\nContent-Length: ${body.size}\r\nConnection: close\r\n\r\n".toByteArray())
                    output.write(body); output.flush()
                }
            } catch (error: Throwable) {
                if (!socket.isClosed) {
                    failure.set(error)
                    socket.close()
                }
            }
        }
    }

    fun assertHealthy() { failure.get()?.let { throw AssertionError("Loopback contract failure", it) } }
    override fun close() {
        socket.close(); worker.shutdownNow()
        worker.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS)
        assertHealthy()
    }
}

class RemoteObservationTransportTest {
    @Test fun submissionTransportFreezesOneEncryptedRequestAndReconcilesOnlyTheOriginalIdentity() = runTest {
        val token = "fixture-mobile-token"
        val identity = RemoteSubmissionIdentity("host", "client", "conversation", "submission", "user", "assistant")
        val digest = "a".repeat(64)
        val request = buildJsonObject {
            put("conversationId", "conversation"); put("expected_host_id", "host"); put("client_id", "client")
            put("submission_id", "submission"); put("user_message_id", "user"); put("assistant_message_id", "assistant")
            put("messages", JsonArray(listOf(buildJsonObject { put("role", "user"); put("content", "原始正文🙂") })))
        }
        val frozen = FrozenRemoteSubmission(identity, digest, request)
        val receipt = buildJsonObject {
            put("protocol_version", 1); put("identity", identity.wire()); put("request_digest", digest)
            put("phase", "prepared"); put("admission", JsonNull); put("rejection_code", JsonNull)
        }
        val seen = java.util.concurrent.CopyOnWriteArrayList<Pair<String, JsonObject>>()
        EncryptedControlLoopback(token) { path, clear ->
            seen += path to clear
            if (path.endsWith("/stream")) {
                assertEquals(frozen.wireRequest(), clear)
                val envelope = MobileCrypto.encryptJsonObject(buildJsonObject { put("type", "submission_receipt"); put("receipt", receipt) }, token).copy(type = "encrypted")
                "text/event-stream" to "data: ${Json.encodeToString(envelope)}\n\ndata: [DONE]\n\n"
            } else {
                assertEquals("/api/mobile/chat/agent/v2/submission", path)
                val result = when (clear.getValue("operation").jsonPrimitive.content) {
                    "prepare" -> {
                        assertEquals(request, clear.getValue("request")); assertFalse(request.containsKey("request_digest"))
                        buildJsonObject { put("identity", identity.wire()); put("request_digest", digest) }
                    }
                    "observe" -> { assertEquals(remoteSubmissionReconciliationRequest(identity, digest), clear); JsonNull }
                    "cancel" -> { assertEquals(remoteSubmissionReconciliationRequest(identity, digest, true), clear); receipt }
                    else -> error("Unexpected submission operation")
                }
                val response = buildJsonObject { put("source_host_id", "host"); put("result", result) }
                "application/json" to Json.encodeToString(MobileCrypto.encryptJsonObject(response, token))
            }
        }.use { server ->
            val client = HttpClient(io.ktor.client.engine.cio.CIO) { install(ContentNegotiation) { json() } }
            try {
                val api = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
                val preparation = api.remoteSubmission(server.baseUrl, token, buildJsonObject {
                    put("operation", "prepare"); put("identity", identity.wire()); put("request", request)
                })
                assertFalse(preparation.getValue("result").jsonObject.containsKey("phase"), "Digest preparation is not a receipt")
                assertEquals(receipt, (api.streamBoundAgentRequest(server.baseUrl, token, frozen).toList().single()
                    as com.nola.data.remote.sse.SseEvent.SubmissionReceipt).receipt)
                assertEquals(JsonNull, api.remoteSubmission(server.baseUrl, token,
                    remoteSubmissionReconciliationRequest(identity, digest)).getValue("result"))
                api.remoteSubmission(server.baseUrl, token, remoteSubmissionReconciliationRequest(identity, digest, true))
                assertEquals(1, seen.count { it.first.endsWith("/stream") })
                assertEquals(4, seen.size)
                assertTrue(seen.filter { it.second["operation"]?.jsonPrimitive?.content in setOf("observe", "cancel") }
                    .all { !it.second.containsKey("request") && !it.second.containsKey("messages") })
                server.assertHealthy()
            } finally { client.close() }
        }
    }

    @Test fun admittedCancellationCarriesExactSubmissionNotOnlyConversation() = runTest {
        val token = "fixture-mobile-token"
        val identity = RemoteSubmissionIdentity("host", "client", "conversation", "original-submission", "original-user", "original-root")
        val digest = "b".repeat(64)
        val requests = java.util.concurrent.atomic.AtomicInteger()
        EncryptedControlLoopback(token) { path, clear ->
            requests.incrementAndGet()
            assertEquals("/api/mobile/chat/agent/v2/cancel", path)
            assertEquals(buildJsonObject { put("identity", identity.wire()); put("request_digest", digest) }, clear)
            "application/json" to Json.encodeToString(MobileCrypto.encryptJsonObject(buildJsonObject { put("cancelRequested", true) }, token))
        }.use { server ->
            val client = HttpClient(io.ktor.client.engine.cio.CIO) { install(ContentNegotiation) { json() } }
            try {
                WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
                    .cancelAdmittedRemoteSubmission(server.baseUrl, token, identity, digest)
                assertEquals(1, requests.get())
                server.assertHealthy()
            } finally { client.close() }
        }
    }

    @Test fun productionControlTransportEncryptsExactObserverAndAuthenticatesTheObservePath() = runTest {
        val token = "fixture-mobile-token"
        val key = RemoteRunKey("conversation", "durable-run", "original-root")
        val request = remoteObservationRequest("host", "client", key.conversationId, key)
        val expected = buildJsonObject { put("source_host_id", "host"); put("result", JsonNull) }
        var calls = 0
        val client = HttpClient(MockEngine { http ->
            calls++
            assertEquals("/api/mobile/chat/agent/v2/observe", http.url.encodedPath)
            assertEquals("aes-gcm-v1", http.headers["X-Nola-Mobile-Crypto"])
            assertNull(http.headers["Authorization"])
            val raw = (http.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()
            assertFalse(raw.contains(key.runId))
            // Rust requires these wire fields; Kotlin defaults must not hide omissions.
            val wire = Json.parseToJsonElement(raw).jsonObject
            assertEquals(JsonPrimitive(1), wire["v"])
            assertEquals(JsonPrimitive("AES-256-GCM"), wire["alg"])
            assertEquals(request, MobileCrypto.decryptJsonObject(Json.decodeFromString<EncryptedEnvelope>(raw), token))
            val headers = mobileAuthHeaders(token, "POST", http.url.toString(),
                http.headers["X-Nola-Mobile-Timestamp"]!!.toLong(), http.headers["X-Nola-Mobile-Nonce"]!!, raw)
            assertEquals(headers["X-Nola-Mobile-Auth"], http.headers["X-Nola-Mobile-Auth"])
            respond(Json.encodeToString(MobileCrypto.encryptJsonObject(expected, token)),
                headers = headersOf("Content-Type", "application/json"))
        }) { install(ContentNegotiation) { json() } }
        try {
            assertEquals(expected, postEncryptedWebHostRequest(client,
                "http://127.0.0.1/api/mobile/chat/agent/v2/observe", token, request))
            assertEquals(1, calls)
        } finally { client.close() }
    }

    @Test fun encryptedControlResponseEnforcesWireBudgetWithoutContentLength() = runTest {
        val token = "fixture-mobile-token"
        val payload = buildJsonObject { put("source_host_id", "host"); put("result", "表单".repeat(100)) }
        val wire = Json.encodeToString(MobileCrypto.encryptJsonObject(payload, token))
        val wireBytes = wire.toByteArray(Charsets.UTF_8).size
        val client = HttpClient(MockEngine {
            respond(wire, headers = headersOf("Content-Type", "application/json"))
        })
        try {
            assertEquals(payload, postEncryptedWebHostRequest(client,
                "http://127.0.0.1/api/mobile/chat/agent/v2/ui/surfaces", token,
                buildJsonObject {}, maximumResponseBytes = wireBytes))
            assertFailsWith<IllegalArgumentException> {
                postEncryptedWebHostRequest(client,
                    "http://127.0.0.1/api/mobile/chat/agent/v2/ui/surfaces", token,
                    buildJsonObject {}, maximumResponseBytes = wireBytes - 1)
            }
        } finally { client.close() }
    }

    @Test fun httpRejectionCannotBeDecodedAsSuccessfulAbsentRun() = runTest {
        for (status in listOf(HttpStatusCode.Unauthorized, HttpStatusCode.NotFound, HttpStatusCode.Conflict)) {
            val client = HttpClient(MockEngine { respond("{}", status = status) })
            try {
                val error = assertFailsWith<WebHostStatusException> {
                    postEncryptedWebHostRequest(client, "http://127.0.0.1/api/mobile/chat/agent/v2/observe",
                        "fixture-token", remoteObservationRequest("host", "client", "conversation"))
                }
                assertEquals(status.value, error.statusCode)
            } finally { client.close() }
        }
    }

    @Test fun actualResumeApiUsesExactOriginalRunAndExposesConflictForObservation() = runTest {
        val token = "fixture-mobile-token"
        val key = RemoteRunKey("conversation", "original-durable-run", "original-root")
        val expected = remoteResumeRequest("host", "client", key)
        var calls = 0
        val client = HttpClient(MockEngine { http ->
            calls++
            assertEquals("/api/mobile/chat/agent/v2/resume", http.url.encodedPath)
            val raw = (http.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()
            val clear = MobileCrypto.decryptJsonObject(Json.decodeFromString<EncryptedEnvelope>(raw), token)
            assertEquals(expected, clear)
            assertFalse(clear.containsKey("submission_id")); assertFalse(clear.containsKey("messages"))
            respond("{}", status = HttpStatusCode.Conflict)
        })
        try {
            val api = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
            val failure = assertFailsWith<SseStatusException> {
                api.resumeRemoteConversation("http://127.0.0.1", token, expected).toList()
            }
            assertEquals(409, failure.statusCode); assertEquals(1, calls)
        } finally { client.close() }
    }
}
