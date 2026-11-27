package com.nola.data.remote.api

import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.remote.dto.VerifiedHostTransport
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WebHostApiServiceImplTest {
    @Test fun expertHistoryQueriesBindHostAndEpochAndRejectMutationCommands() = kotlinx.coroutines.runBlocking {
        val command = JsonObject(mapOf("type" to JsonPrimitive("peer_exchanges"), "conversation_id" to JsonPrimitive("parent")))
        val result = JsonObject(mapOf("conversation_id" to JsonPrimitive("parent")))
        var host = "owned-host"
        var epoch = "owned-epoch"
        var requests = 0
        val api = java.lang.reflect.Proxy.newProxyInstance(WebHostApiService::class.java.classLoader,
            arrayOf(WebHostApiService::class.java)) { _, method, args ->
            check(method.name == "agentRoomCommand")
            val request = args[2] as JsonObject
            assertEquals(JsonPrimitive("owned-host"), request["expected_host_id"])
            assertEquals(JsonPrimitive("owned-epoch"), request["expected_source_store_epoch"])
            assertEquals(command, request["command"])
            requests++
            JsonObject(mapOf("host_id" to JsonPrimitive(host), "source_store_epoch" to JsonPrimitive(epoch), "result" to result))
        } as WebHostApiService
        val provider = com.nola.llm.provider.WebAgentProvider(api, com.nola.llm.provider.LocaleWebAgentSummaryTextSource)
        val config = com.nola.llm.provider.WebAgentProvider.Config("http://127.0.0.1", "fixture", expectedHostId = host)
        assertEquals(result, provider.readExpertHistoryCommand(command, config, epoch))
        host = "different-host"
        assertTrue(runCatching { provider.readExpertHistoryCommand(command, config, "owned-epoch") }.isFailure)
        host = "owned-host"
        epoch = "different-epoch"
        assertTrue(runCatching { provider.readExpertHistoryCommand(command, config, "owned-epoch") }.isFailure)
        val mutation = JsonObject(mapOf("type" to JsonPrimitive("configure_agent")))
        assertTrue(runCatching { provider.readExpertHistoryCommand(mutation, config, "owned-epoch") }.isFailure)
        assertEquals(3, requests)
    }

    @Test fun tailcatStartupErrorsDoNotExposeNativeMessages() {
        assertEquals("native_host_inspection", tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception("encrypted Nola Host inspection failed")), "native_connect"))
        assertEquals("native_host_dns", tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception("encrypted Nola Host inspection failed (dns)")), "native_connect"))
        val secret = "fixture-secret-address-psk"
        val bounded = tailcatStartupErrorCode(java.lang.reflect.InvocationTargetException(Exception(secret)), "native_connect")
        assertEquals("native_connect:InvocationTargetException", bounded)
        assertFalse(bounded.contains(secret))
        assertEquals("sdk_binding", tailcatStartupErrorCode(NoSuchMethodException(secret), "native_connect"))
    }

    @Test
    fun webHostTransportPolicyLimitsHttpToPrivateNetworks() {
        assertFalse(isAllowedWebHostTransport("http://10.0.0.1.example.com:5001"))
        assertEquals(true, isAllowedWebHostTransport("https://host.example"))
        assertEquals(true, isAllowedWebHostTransport("http://127.0.0.1:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://10.0.2.2:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://172.31.4.2:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://192.168.1.8:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://nola.local:5001"))
        assertEquals(true, isAllowedWebHostTransport("http://[fd00::1]:5001"))
        assertEquals(false, isAllowedWebHostTransport("http://example.com:5001"))
        assertEquals(false, isAllowedWebHostTransport("http://8.8.8.8:5001"))
        assertEquals(false, isAllowedWebHostTransport("https://user:secret@example.com"))
    }

    @Test
    fun certificateFingerprintNormalizationAcceptsCommonDisplayFormats() {
        val raw = "SHA256/AA:BB:" + "CC".repeat(30)

        assertEquals("aabb" + "cc".repeat(30), normalizeCertificateSha256(raw))
        assertEquals("", normalizeCertificateSha256("  "))
        assertEquals(null, normalizeCertificateSha256("not-a-fingerprint"))
    }

    @Test
    fun mobileAuthProofBindsRequestPathWithoutSendingBearerToken() {
        val first = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )
        val repeated = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )
        val otherPath = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/other",
            timestamp = 100,
            nonce = "mobile-nonce-123456"
        )

        assertEquals(first, repeated)
        assertEquals("100", first["X-Nola-Mobile-Timestamp"])
        assertEquals("mobile-nonce-123456", first["X-Nola-Mobile-Nonce"])
        assertEquals(64, first["X-Nola-Mobile-Body-SHA256"]?.length)
        assertFalse(first.containsKey("Authorization"))
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherPath["X-Nola-Mobile-Auth"])
        val otherBody = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
            body = "different"
        )
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherBody["X-Nola-Mobile-Auth"])
    }

    @Test
    fun webHostMobileUrlNormalizesCopiedMobileEndpoints() {
        assertEquals(
            "http://192.168.1.8:5001/api/mobile/chat/conversations/c1/messages",
            webHostMobileUrl(
                "http://192.168.1.8:5001/api/mobile/login",
                "chat/conversations/c1/messages"
            )
        )
        assertEquals(
            "http://192.168.1.8:5001/api/mobile/agent/settings",
            webHostMobileUrl(
                "192.168.1.8:5001/api/agent/settings?token=copy",
                "/agent/settings"
            )
        )
        assertEquals(
            "https://nola.local/api/mobile/chat/conversations/c1/messages",
            webHostMobileUrl(
                "https://nola.local/api",
                "chat/conversations/c1/messages"
            )
        )
        assertEquals(
            "https://nola.local/api/mobile/agent/settings",
            webHostMobileUrl("https://nola.local/v1", "agent/settings")
        )
    }

    @Test
    fun webHostControlPlaneUrlUsesOriginInsteadOfApiBasePath() {
        assertEquals(
            "https://nola.local:7443/models?tab=api",
            webHostControlPlaneUrl("https://nola.local:7443/api/mobile/login", "/models?tab=api")
        )
        assertEquals(
            "https://nola.local:7443/settings#channels",
            webHostControlPlaneUrl(
                "https://nola.local:7443/api/mobile/agent/settings",
                "/settings#channels"
            )
        )
        assertEquals(
            "https://nola.local/settings/channels?tab=active#telegram",
            webHostControlPlaneUrl(
                "https://nola.local/v1",
                "settings/channels?tab=active#telegram"
            )
        )
    }

    @Test
    fun browserHandoffPathKeepsSingleUseCodeOnHostOrigin() {
        assertEquals(
            "https://nola.local:7443/auth/mobile-handoff/one-time-code",
            webHostControlPlaneUrl(
                "https://nola.local:7443/api/mobile/agent/settings",
                "/auth/mobile-handoff/one-time-code"
            )
        )
    }

    @Test
    fun webHostMobileUrlPreservesConfiguredOriginIncludingExplicitEmulatorAlias() {
        assertEquals(
            "http://127.0.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://127.0.0.1:5001/api/mobile/login", "agent/settings")
        )
        assertEquals(
            "http://localhost:5001/api/mobile/agent/settings",
            webHostMobileUrl("localhost:5001", "/agent/settings")
        )
        assertEquals(
            "http://[::1]:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://[::1]:5001/api", "agent/settings")
        )
        assertEquals(
            "HTTP://127.0.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("HTTP://127.0.0.1:5001/api", "agent/settings")
        )
        assertEquals(
            "http://10.0.2.2:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://10.0.2.2:5001", "agent/settings")
        )
        assertEquals(
            "http://127.999.0.1:5001/api/mobile/agent/settings",
            webHostMobileUrl("http://127.999.0.1:5001/api", "agent/settings")
        )
    }

    @Test
    fun loopbackToolUrlsCanBeRewrittenAgainstReachableHost() {
        assertEquals(
            "http://192.168.1.8:3000/app",
            rewriteLoopbackHttpUrlForWebHost(
                "http://127.0.0.1:3000/app",
                "http://192.168.1.8:5001/api/mobile/login"
            )
        )
        assertEquals(
            "http://127.0.0.1:5001/static/call.png",
            rewriteLoopbackHttpUrlForWebHost(
                "http://localhost/static/call.png",
                "http://127.0.0.1:5001"
            )
        )
        assertEquals(
            "http://localhost/static/call.png",
            rewriteLoopbackHttpUrlForWebHost("http://localhost/static/call.png", "")
        )
        assertEquals(
            "https://example.com/static/call.png",
            rewriteLoopbackHttpUrlForWebHost(
                "https://example.com/static/call.png",
                "http://127.0.0.1:5001"
            )
        )
    }

    @Test
    fun requestBodyCarriesMobileAgentProtocolFields() {
        val bridge = JsonObject(mapOf("protocol_version" to JsonPrimitive(1)))

        val body = buildWebHostAgentRequestBody(
            messages = listOf(
                JsonObject(
                    mapOf(
                        "role" to JsonPrimitive("user"),
                        "content" to JsonPrimitive("hello"),
                    ),
                ),
            ),
            conversationId = "conversation-1",
            allowLocalDevTools = true,
            maxIterations = 30,
            timezone = "Asia/Shanghai",
            language = "zh-Hans",
            agentMode = true,
            permissionProfile = AndroidAgentPermissionMode.AllowAll,
            responseStyle = "casual",
            customSystemPrompt = "Prefer concise replies.",
            nativeAppHostId = "desktop-host",
            expectedHostId = "cli-host-paired-1234",
            clientId = "mobile-client-1234",
            deviceBridge = bridge
        )

        assertEquals("Prefer concise replies.", body["custom_system_prompt"])
        assertEquals("desktop-host", body["native_app_host_id"])
        assertEquals("conversation-1", body["conversationId"])
        assertEquals("zh-Hans", body["language"])
        assertEquals("cli-host-paired-1234", body["expected_host_id"])
        assertEquals("mobile-client-1234", body["client_id"])
        assertEquals(bridge, body["device_bridge"])
        assertEquals("allow_all", body["permission_profile"])
    }

    @Test
    fun requestBodyOmitsBlankPromptMetadata() {
        val body = buildWebHostAgentRequestBody(
            messages = listOf(
                JsonObject(
                    mapOf(
                        "role" to JsonPrimitive("user"),
                        "content" to JsonPrimitive("hello"),
                    ),
                ),
            ),
            conversationId = null,
            allowLocalDevTools = true,
            maxIterations = 30,
            timezone = "Asia/Shanghai",
            language = "   ",
            agentMode = true,
            responseStyle = "   ",
            customSystemPrompt = "   ",
            nativeAppHostId = "   ",
            expectedHostId = "   ",
            clientId = "   "
        )

        assertFalse(body.containsKey("custom_system_prompt"))
        assertFalse(body.containsKey("response_style"))
        assertFalse(body.containsKey("native_app_host_id"))
        assertFalse(body.containsKey("conversationId"))
        assertEquals("cautious", body["permission_profile"])
        assertFalse(body.containsKey("language"))
        assertFalse(body.containsKey("expected_host_id"))
        assertFalse(body.containsKey("client_id"))
    }

    @Test
    fun generativeUiOperationsIncludePreviewWithDedicatedResponseLimit() {
        assertTrue("preview" in GENERATIVE_UI_OPERATIONS)
        assertEquals(2 * 1024 * 1024, generativeUiMaxResponseBytes("preview"))
        assertEquals(16 * 1024 * 1024, generativeUiMaxResponseBytes("surfaces"))
    }

    @Test
    fun v2StreamMessageAttachesMobileAuthAndEncryptsRequestAndSseFramesOverLanHttp() = runBlocking {
        val token = "lan-shared-secret-token"
        val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
        val capturedRequestLine = java.util.concurrent.atomic.AtomicReference("")
        val capturedHeaders = java.util.concurrent.atomic.AtomicReference<Map<String, String>>(emptyMap())
        val capturedBody = java.util.concurrent.atomic.AtomicReference("")
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))

        executor.submit {
            server.accept().use { socket ->
                val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
                capturedRequestLine.set(input.readLine().orEmpty())
                val headers = mutableMapOf<String, String>()
                while (true) {
                    val line = input.readLine() ?: break
                    if (line.isEmpty()) break
                    val sep = line.indexOf(':')
                    if (sep > 0) {
                        headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
                    }
                }
                capturedHeaders.set(headers)
                val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
                val chars = CharArray(bodyLength)
                var offset = 0
                while (offset < bodyLength) {
                    val read = input.read(chars, offset, bodyLength - offset)
                    if (read < 0) break
                    offset += read
                }
                capturedBody.set(String(chars, 0, offset))

                val deltaEnvelope = MobileCrypto.encryptJsonObject(
                    buildJsonObject {
                        put("type", "text_delta")
                        put("delta", "hello encrypted")
                    },
                    token,
                ).copy(type = "encrypted")
                val deviceToolEnvelope = MobileCrypto.encryptJsonObject(
                    buildJsonObject {
                        put("type", "device_tool_request")
                        put("request_id", "dt-1")
                        put("nonce", "nonce-1")
                        put("host_id", "host-1")
                        put("client_id", "client-1")
                        put("session_id", "conv-42")
                        put("tool_name", "location_current")
                        put("capability", "location")
                        put("arguments", buildJsonObject { put("precision", "coarse") })
                        put("expires_at", 9_999_999_999L)
                        put("protocol_version", 1)
                    },
                    token,
                ).copy(type = "encrypted")

                val frames = listOf(
                    "data: ${json.encodeToString(deltaEnvelope)}\n\n",
                    "data: ${json.encodeToString(deviceToolEnvelope)}\n\n",
                    "data: [DONE]\n\n",
                )
                val payloadBytes = frames.joinToString("").toByteArray(Charsets.UTF_8)
                val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
                output.write(
                    "HTTP/1.1 200 OK\r\n" +
                        "Content-Type: text/event-stream\r\n" +
                        "Connection: close\r\n" +
                        "Content-Length: ${payloadBytes.size}\r\n\r\n"
                )
                output.write(String(payloadBytes, Charsets.UTF_8))
                output.flush()
            }
        }

        val client = HttpClient(CIO)
        try {
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
            val events = service.v2StreamMessage(
                hostBaseUrl = "http://127.0.0.1:${server.localPort}",
                token = token,
                conversationId = "conv-42",
                content = "secret prompt",
                deviceBridge = buildJsonObject { put("protocol_version", 1) },
            ).toList()

            assertTrue(capturedRequestLine.get().startsWith("POST /api/mobile/chat/conversations/conv-42/messages "))
            val headers = capturedHeaders.get()
            assertEquals("aes-gcm-v1", headers["x-nola-mobile-crypto"])
            assertFalse("LAN stream must not send plaintext Bearer header", headers.containsKey("authorization"))
            assertNotNull(headers["x-nola-mobile-auth"])
            assertNotNull(headers["x-nola-mobile-timestamp"])
            assertNotNull(headers["x-nola-mobile-nonce"])
            assertEquals(64, headers["x-nola-mobile-body-sha256"]?.length)

            val rawWireBody = capturedBody.get()
            assertFalse("Request body must be encrypted on LAN HTTP", rawWireBody.contains("secret prompt"))
            val decodedRequest = MobileCrypto.decryptJsonObject(
                json.decodeFromString<EncryptedEnvelope>(rawWireBody),
                token,
            )
            assertEquals(JsonPrimitive("secret prompt"), decodedRequest["content"])
            assertEquals(JsonPrimitive(1), (decodedRequest["device_bridge"] as? JsonObject)?.get("protocol_version"))

            assertEquals(2, events.size)
            assertEquals(SseEvent.V2TextDelta("hello encrypted"), events[0])
            val deviceReq = events[1] as SseEvent.DeviceToolRequest
            assertEquals("dt-1", deviceReq.id)
            assertEquals("conv-42", deviceReq.sessionId)
            assertEquals("location_current", deviceReq.toolName)
            assertEquals("coarse", deviceReq.args["precision"])
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun remoteHostEndpointsRouteToExpectedPathsWithMobileAuthAndCrypto() = runBlocking {
        val token = "lan-endpoint-token"
        val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
        val recordedRequests = CopyOnWriteArrayList<Pair<String, Map<String, String>>>()
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))

        executor.submit {
            repeat(7) {
                server.accept().use { socket ->
                    val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
                    val requestLine = input.readLine().orEmpty()
                    val headers = mutableMapOf<String, String>()
                    while (true) {
                        val line = input.readLine() ?: break
                        if (line.isEmpty()) break
                        val sep = line.indexOf(':')
                        if (sep > 0) {
                            headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
                        }
                    }
                    val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
                    val chars = CharArray(bodyLength)
                    var offset = 0
                    while (offset < bodyLength) {
                        val read = input.read(chars, offset, bodyLength - offset)
                        if (read < 0) break
                        offset += read
                    }
                    recordedRequests.add(requestLine to headers)

                    val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
                    if (requestLine.contains("/tasks/events")) {
                        val checkpointEnvelope = MobileCrypto.encryptJsonObject(
                            buildJsonObject {
                                put("type", "task_stream_checkpoint")
                                put("conversationId", "conv-9")
                                put("cursor", 12)
                            },
                            token,
                        ).copy(type = "encrypted")
                        val sseBody = "data: ${json.encodeToString(checkpointEnvelope)}\n\ndata: [DONE]\n\n"
                        val sseBytes = sseBody.toByteArray(Charsets.UTF_8)
                        output.write(
                            "HTTP/1.1 200 OK\r\n" +
                                "Content-Type: text/event-stream\r\n" +
                                "Connection: close\r\n" +
                                "Content-Length: ${sseBytes.size}\r\n\r\n"
                        )
                        output.write(sseBody)
                    } else {
                        val replyObj = if (requestLine.contains("/agent/settings")) {
                            buildJsonObject {
                                put("host_id", "host-9")
                                put("max_iterations", 16)
                            }
                        } else {
                            buildJsonObject {
                                put("ok", true)
                                put("source_host_id", "host-9")
                            }
                        }
                        val encryptedReply = json.encodeToString(
                            MobileCrypto.encryptJsonObject(replyObj, token).copy(type = "encrypted")
                        )
                        val replyBytes = encryptedReply.toByteArray(Charsets.UTF_8)
                        output.write(
                            "HTTP/1.1 200 OK\r\n" +
                                "Content-Type: application/json\r\n" +
                                "Connection: close\r\n" +
                                "Content-Length: ${replyBytes.size}\r\n\r\n"
                        )
                        output.write(encryptedReply)
                    }
                    output.flush()
                }
            }
        }

        val client = HttpClient(CIO)
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))

            service.submitAgentDeviceToolResult(
                hostBaseUrl = baseUrl,
                token = token,
                payload = mapOf("session_id" to "conv-9", "request_id" to "dt-9", "output" to "ok"),
            )
            service.goalControl(
                hostBaseUrl = baseUrl,
                token = token,
                request = buildJsonObject { put("conversation_id", "conv-9"); put("action", "start") },
            )
            service.auxiliaryCommand(
                hostBaseUrl = baseUrl,
                token = token,
                action = "invoke",
                request = buildJsonObject { put("room_id", "room-1") },
            )
            service.conversationProgress(
                hostBaseUrl = baseUrl,
                token = token,
                request = buildJsonObject { put("conversation_id", "conv-9") },
            )
            service.observeRemoteConversation(
                hostBaseUrl = baseUrl,
                token = token,
                request = buildJsonObject { put("conversation_id", "conv-9") },
            )
            val taskEvents = service.streamAgentTaskEvents(
                hostBaseUrl = baseUrl,
                token = token,
                conversationId = "conv-9",
                afterCursor = 7L,
            ).toList()
            val settings = service.getAgentSettings(
                hostBaseUrl = baseUrl,
                token = token,
            )

            assertEquals(7, recordedRequests.size)
            assertTrue(recordedRequests[0].first.startsWith("POST /api/mobile/chat/conversations/conv-9/device-tool "))
            assertTrue(recordedRequests[1].first.startsWith("POST /api/mobile/chat/conversations/conv-9/goal "))
            assertTrue(recordedRequests[2].first.startsWith("POST /api/mobile/auxiliary/invoke "))
            assertTrue(recordedRequests[3].first.startsWith("POST /api/mobile/chat/conversations/conv-9/progress?conversation_id=conv-9 "))
            assertTrue(recordedRequests[4].first.startsWith("POST /api/mobile/chat/conversations/conv-9/observe "))
            assertTrue(recordedRequests[5].first.startsWith("GET /api/mobile/chat/conversations/conv-9/tasks/events?afterCursor=7 "))
            assertTrue(recordedRequests[6].first.startsWith("GET /api/mobile/agent/settings "))

            for ((line, headers) in recordedRequests) {
                assertEquals("Missing aes-gcm-v1 on $line", "aes-gcm-v1", headers["x-nola-mobile-crypto"])
                assertNotNull("Missing HMAC proof on $line", headers["x-nola-mobile-auth"])
                assertFalse("Plaintext Bearer token must not be sent on $line", headers.containsKey("authorization"))
            }

            assertEquals(listOf(SseEvent.TaskStreamCheckpoint("conv-9", 12L)), taskEvents)
            assertEquals(VerifiedHostTransport.DirectWithoutTailcat, settings.verifiedTransport)
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun pairedHostControlMethodsPostFullEncryptedRequestBodies() = runBlocking {
        val token = "lan-paired-body-token"
        val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
        val recorded = CopyOnWriteArrayList<Triple<String, Map<String, String>, JsonObject>>()
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val expectedCalls = 11

        executor.submit {
            repeat(expectedCalls) {
                server.accept().use { socket ->
                    val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
                    val requestLine = input.readLine().orEmpty()
                    val headers = mutableMapOf<String, String>()
                    while (true) {
                        val line = input.readLine() ?: break
                        if (line.isEmpty()) break
                        val sep = line.indexOf(':')
                        if (sep > 0) {
                            headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
                        }
                    }
                    val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
                    val chars = CharArray(bodyLength)
                    var offset = 0
                    while (offset < bodyLength) {
                        val read = input.read(chars, offset, bodyLength - offset)
                        if (read < 0) break
                        offset += read
                    }
                    val rawBody = String(chars, 0, offset)
                    val decoded = MobileCrypto.decryptJsonObject(
                        json.decodeFromString<EncryptedEnvelope>(rawBody),
                        token,
                    )
                    recorded.add(Triple(requestLine, headers, decoded))

                    val encryptedReply = json.encodeToString(
                        MobileCrypto.encryptJsonObject(
                            buildJsonObject { put("ok", true); put("source_host_id", "host-1") },
                            token,
                        ).copy(type = "encrypted")
                    )
                    val replyBytes = encryptedReply.toByteArray(Charsets.UTF_8)
                    val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
                    output.write(
                        "HTTP/1.1 200 OK\r\n" +
                            "Content-Type: application/json\r\n" +
                            "Connection: close\r\n" +
                            "Content-Length: ${replyBytes.size}\r\n\r\n"
                    )
                    output.write(encryptedReply)
                    output.flush()
                }
            }
        }

        val client = HttpClient(CIO)
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))

            service.permissionConfiguration(baseUrl, token, buildJsonObject {
                put("operation", "read")
                put("expected_host_id", "host-1")
                put("conversation_id", "conv-1")
            })
            service.submissionFrontier(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1")
                put("client_id", "client-1")
                put("conversation_id", "conv-1")
            })
            service.conversationProgress(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1")
                put("conversation_id", "conv-1")
            })
            service.observeRemoteConversation(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1")
                put("client_id", "client-1")
                put("conversation_id", "conv-1")
            })
            service.generativeUi(baseUrl, token, "surfaces", buildJsonObject {
                put("expected_host_id", "host-1")
                put("conversation_id", "conv-1")
            })
            service.generativeUi(baseUrl, token, "queued-input", buildJsonObject {
                put("expected_host_id", "host-1")
                put("client_id", "client-1")
                put("conversation_id", "conv-1")
            })
            service.readAssistantJournal(baseUrl, token, buildJsonObject {
                put("operation", "open_snapshot")
                put("at", buildJsonObject {
                    put("identity", buildJsonObject { put("conversation_id", "conv-at") })
                })
            })
            service.readAssistantJournal(baseUrl, token, buildJsonObject {
                put("operation", "record_chunk")
                put("snapshot", buildJsonObject {
                    put("at", buildJsonObject {
                        put("identity", buildJsonObject { put("conversation_id", "conv-snap") })
                    })
                })
            })
            service.remoteSubmission(baseUrl, token, buildJsonObject {
                put("operation", "cancel")
                put("identity", buildJsonObject { put("conversation_id", "conv-sub") })
                put("request_digest", "a".repeat(64))
            })
            service.taskCancellationCommand(baseUrl, token, buildJsonObject {
                put("hostId", "host-1")
                put("conversationId", "conv-task")
                put("parentRunId", "run-1")
                put("taskId", "task-42")
            }, receiptOnly = true)
            service.taskGuidanceCommand(baseUrl, token, buildJsonObject {
                put("hostId", "host-1")
                put("conversationId", "conv-task")
                put("parentRunId", "run-1")
                put("taskId", "task-42")
                put("messageId", "msg-7")
                put("message", "focus on tests")
            }, receiptOnly = false)

            assertEquals(expectedCalls, recorded.size)
            assertTrue(recorded[0].first.startsWith("POST /api/mobile/chat/conversations/conv-1/configuration "))
            assertEquals(JsonPrimitive("read"), recorded[0].third["operation"])
            assertEquals(JsonPrimitive("host-1"), recorded[0].third["expected_host_id"])

            assertTrue(recorded[1].first.startsWith("POST /api/mobile/chat/conversations/conv-1/configuration "))
            assertEquals(JsonPrimitive("host-1"), recorded[1].third["expected_host_id"])
            assertEquals(JsonPrimitive("client-1"), recorded[1].third["client_id"])

            assertTrue(recorded[2].first.startsWith("POST /api/mobile/chat/conversations/conv-1/progress?expected_host_id=host-1&conversation_id=conv-1 "))
            assertEquals(JsonPrimitive("host-1"), recorded[2].third["expected_host_id"])

            assertTrue(recorded[3].first.startsWith("POST /api/mobile/chat/conversations/conv-1/observe "))
            assertEquals(JsonPrimitive("host-1"), recorded[3].third["expected_host_id"])
            assertEquals(JsonPrimitive("client-1"), recorded[3].third["client_id"])

            assertTrue(recorded[4].first.startsWith("POST /api/mobile/chat/conversations/conv-1/ui-surfaces "))
            assertEquals(JsonPrimitive("host-1"), recorded[4].third["expected_host_id"])

            assertTrue(recorded[5].first.startsWith("POST /api/mobile/chat/conversations/conv-1/events "))
            assertEquals(JsonPrimitive("host-1"), recorded[5].third["expected_host_id"])

            assertTrue(recorded[6].first.startsWith("POST /api/mobile/chat/conversations "))
            assertEquals(JsonPrimitive("open_snapshot"), recorded[6].third["operation"])
            assertEquals(JsonPrimitive("conv-at"), recorded[6].third["conversation_id"])

            assertTrue(recorded[7].first.startsWith("POST /api/mobile/chat/conversations "))
            assertEquals(JsonPrimitive("record_chunk"), recorded[7].third["operation"])
            assertEquals(JsonPrimitive("conv-snap"), recorded[7].third["conversation_id"])

            assertTrue(recorded[8].first.startsWith("POST /api/mobile/chat/conversations/conv-sub "))
            assertEquals(JsonPrimitive("cancel"), recorded[8].third["operation"])

            assertTrue(recorded[9].first.startsWith("POST /api/mobile/chat/conversations/conv-task/tasks/task-42/stop "))
            assertEquals(JsonPrimitive("task-42"), recorded[9].third["task_id"])
            assertEquals(JsonPrimitive("run-1"), recorded[9].third["parent_run_id"])
            assertEquals(JsonPrimitive(true), recorded[9].third["receipt_only"])
            assertEquals(JsonPrimitive("host-1"), recorded[9].third["expected_host_id"])

            assertTrue(recorded[10].first.startsWith("POST /api/mobile/chat/conversations/conv-task/messages "))
            assertEquals("application/json", recorded[10].second["accept"])
            assertEquals(JsonPrimitive("focus on tests"), recorded[10].third["message"])
            assertEquals(JsonPrimitive(false), recorded[10].third["receipt_only"])
            assertEquals(JsonPrimitive("host-1"), recorded[10].third["expected_host_id"])

            for ((line, headers, _) in recorded) {
                assertTrue("Expected JSON Content-Type on $line", headers["content-type"]?.startsWith("application/json") == true)
                assertEquals("Expected JSON Accept on $line", "application/json", headers["accept"])
                assertEquals("Missing aes-gcm-v1 on $line", "aes-gcm-v1", headers["x-nola-mobile-crypto"])
            }
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun boundedWebHostErrorCodeReadsCodeAndErrorKeys() {
        assertEquals(
            "unknown_transport_operation",
            boundedWebHostErrorCode(buildJsonObject { put("code", "unknown_transport_operation") }),
        )
        assertEquals(
            "unknown_transport_operation",
            boundedWebHostErrorCode(buildJsonObject { put("error", "unknown_transport_operation") }),
        )
    }
}
