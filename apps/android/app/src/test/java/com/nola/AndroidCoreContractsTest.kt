// SPDX-License-Identifier: Apache-2.0
package com.nola

import androidx.compose.ui.graphics.Color
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.remote.api.GENERATIVE_UI_OPERATIONS
import com.nola.data.remote.api.WebHostApiService
import com.nola.data.remote.api.WebHostApiServiceImpl
import com.nola.data.remote.api.WebHostHttpClientFactory
import com.nola.data.remote.api.boundedWebHostErrorCode
import com.nola.data.remote.api.buildWebHostAgentRequestBody
import com.nola.data.remote.api.generativeUiMaxResponseBytes
import com.nola.data.remote.api.isAllowedWebHostTransport
import com.nola.data.remote.api.mobileAuthHeaders
import com.nola.data.remote.api.normalizeCertificateSha256
import com.nola.data.remote.api.rewriteLoopbackHttpUrlForWebHost
import com.nola.data.remote.api.tailcatStartupErrorCode
import com.nola.data.remote.api.webHostControlPlaneUrl
import com.nola.data.remote.api.webHostMobileUrl
import com.nola.data.remote.dto.VerifiedHostTransport
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.decodeAnswerSourcesFromMeta
import com.nola.domain.projection.CanonicalConversationProjector
import com.nola.domain.projection.CapabilityOrigin
import com.nola.domain.projection.CitationRange
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.ConversationProjectionV1Decoder
import com.nola.domain.projection.MemoryNamespace
import com.nola.domain.projection.PublicPartSemanticsDecoder
import com.nola.domain.projection.SourceLocation
import com.nola.domain.projection.ToolOutputOwner
import com.nola.domain.projection.asLegacyLongOrNull
import com.nola.domain.projection.cardFacts
import com.nola.domain.projection.sourceCardFacts
import com.nola.llm.provider.LocaleWebAgentSummaryTextSource
import com.nola.llm.provider.WebAgentProvider
import com.nola.ui.components.NolaColors
import com.nola.ui.theme.DarkColors
import com.nola.ui.theme.LightColors
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.File
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Proxy
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import javax.xml.parsers.DocumentBuilderFactory
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.test.assertFails
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * High-signal JVM contract suite covering genuine non-UI invariants:
 * 1. Paired WebHost HMAC authentication, AES-GCM transport crypto, and POST body preservation.
 * 2. Canonical PublicPartSemantics decoding & leakage guardrails (including `"core"` origin kind).
 * 3. Shared Warm Paper / Warm Ink design token parity, WCAG AA contrast, and 6-locale parity.
 */
class AndroidCoreContractsTest {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    // -------------------------------------------------------------------------
    // 1. WebHost Mobile Auth, Crypto & Paired-Host Endpoint Contracts
    // -------------------------------------------------------------------------

    @Test
    fun mobileAuthProofAndTransportPolicyEnforcePrivateLanAndPathBinding() {
        val first = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
        )
        val repeated = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
        )
        val otherPath = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/other",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
        )
        val otherBody = mobileAuthHeaders(
            token = "shared-token",
            method = "POST",
            url = "http://host.local/api/chat/conversations/c1/messages",
            timestamp = 100,
            nonce = "mobile-nonce-123456",
            body = "different",
        )

        assertEquals(first, repeated)
        assertEquals("100", first["X-Nola-Mobile-Timestamp"])
        assertEquals("mobile-nonce-123456", first["X-Nola-Mobile-Nonce"])
        assertEquals(64, first["X-Nola-Mobile-Body-SHA256"]?.length)
        assertFalse(first.containsKey("Authorization"))
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherPath["X-Nola-Mobile-Auth"])
        assertNotEquals(first["X-Nola-Mobile-Auth"], otherBody["X-Nola-Mobile-Auth"])

        // Private LAN HTTP allowed; public plaintext HTTP and userinfo rejected.
        assertTrue(isAllowedWebHostTransport("https://host.example"))
        assertTrue(isAllowedWebHostTransport("http://127.0.0.1:5001"))
        assertTrue(isAllowedWebHostTransport("http://10.0.2.2:5001"))
        assertTrue(isAllowedWebHostTransport("http://172.31.4.2:5001"))
        assertTrue(isAllowedWebHostTransport("http://192.168.1.8:5001"))
        assertTrue(isAllowedWebHostTransport("http://nola.local:5001"))
        assertTrue(isAllowedWebHostTransport("http://[fd00::1]:5001"))
        assertFalse(isAllowedWebHostTransport("http://10.0.0.1.example.com:5001"))
        assertFalse(isAllowedWebHostTransport("http://example.com:5001"))
        assertFalse(isAllowedWebHostTransport("http://8.8.8.8:5001"))
        assertFalse(isAllowedWebHostTransport("https://user:secret@example.com"))

        // URL normalization & certificate pinning.
        assertEquals(
            "http://192.168.1.8:5001/api/mobile/chat/conversations/c1/messages",
            webHostMobileUrl("http://192.168.1.8:5001/api/mobile/login", "chat/conversations/c1/messages"),
        )
        assertEquals(
            "https://nola.local:7443/models?tab=api",
            webHostControlPlaneUrl("https://nola.local:7443/api/mobile/login", "/models?tab=api"),
        )
        assertEquals(
            "http://192.168.1.8:3000/app",
            rewriteLoopbackHttpUrlForWebHost("http://127.0.0.1:3000/app", "http://192.168.1.8:5001/api/mobile/login"),
        )
        assertEquals("aabb" + "cc".repeat(30), normalizeCertificateSha256("SHA256/AA:BB:" + "CC".repeat(30)))
        assertNull(normalizeCertificateSha256("not-a-fingerprint"))

        // Request body protocol fields, error codes, and Tailcat error redaction.
        val bridge = JsonObject(mapOf("protocol_version" to JsonPrimitive(1)))
        val populated = buildWebHostAgentRequestBody(
            messages = listOf(JsonObject(mapOf("role" to JsonPrimitive("user"), "content" to JsonPrimitive("hello")))),
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
            deviceBridge = bridge,
        )
        assertEquals("Prefer concise replies.", populated["custom_system_prompt"])
        assertEquals("cli-host-paired-1234", populated["expected_host_id"])
        assertEquals("allow_all", populated["permission_profile"])
        assertEquals(bridge, populated["device_bridge"])

        val minimal = buildWebHostAgentRequestBody(
            messages = listOf(JsonObject(mapOf("role" to JsonPrimitive("user"), "content" to JsonPrimitive("hello")))),
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
            clientId = "   ",
        )
        assertFalse(minimal.containsKey("custom_system_prompt"))
        assertFalse(minimal.containsKey("expected_host_id"))
        assertEquals("cautious", minimal["permission_profile"])

        assertTrue("preview" in GENERATIVE_UI_OPERATIONS)
        assertEquals(2 * 1024 * 1024, generativeUiMaxResponseBytes("preview"))
        assertEquals("unknown_transport_operation", boundedWebHostErrorCode(buildJsonObject { put("code", "unknown_transport_operation") }))
        assertEquals("unknown_transport_operation", boundedWebHostErrorCode(buildJsonObject { put("error", "unknown_transport_operation") }))
        val secret = "fixture-secret-address-psk"
        val bounded = tailcatStartupErrorCode(InvocationTargetException(Exception(secret)), "native_connect")
        assertEquals("native_connect:InvocationTargetException", bounded)
        assertFalse(bounded.contains(secret))
    }

    @Test
    fun expertHistoryQueriesBindHostAndEpochAndRejectMutations() = runBlocking {
        val command = JsonObject(mapOf("type" to JsonPrimitive("peer_exchanges"), "conversation_id" to JsonPrimitive("parent")))
        val result = JsonObject(mapOf("conversation_id" to JsonPrimitive("parent")))
        var host = "owned-host"
        var epoch = "owned-epoch"
        var requests = 0
        val api = Proxy.newProxyInstance(
            WebHostApiService::class.java.classLoader,
            arrayOf(WebHostApiService::class.java),
        ) { _, method, args ->
            check(method.name == "agentRoomCommand")
            val request = args[2] as JsonObject
            assertEquals(JsonPrimitive("owned-host"), request["expected_host_id"])
            assertEquals(JsonPrimitive("owned-epoch"), request["expected_source_store_epoch"])
            requests++
            JsonObject(mapOf("host_id" to JsonPrimitive(host), "source_store_epoch" to JsonPrimitive(epoch), "result" to result))
        } as WebHostApiService
        val provider = WebAgentProvider(api, LocaleWebAgentSummaryTextSource)
        val config = WebAgentProvider.Config("http://127.0.0.1", "fixture", expectedHostId = host)
        assertEquals(result, provider.readExpertHistoryCommand(command, config, epoch))
        host = "different-host"
        assertTrue(runCatching { provider.readExpertHistoryCommand(command, config, "owned-epoch") }.isFailure)
        host = "owned-host"
        epoch = "different-epoch"
        assertTrue(runCatching { provider.readExpertHistoryCommand(command, config, "owned-epoch") }.isFailure)
        assertTrue(runCatching { provider.readExpertHistoryCommand(JsonObject(mapOf("type" to JsonPrimitive("configure_agent"))), config, "owned-epoch") }.isFailure)
        assertEquals(3, requests)
    }

    @Test
    fun v2StreamMessageEncryptsRequestAndSseFramesWithHmacOverLanHttp() = runBlocking {
        val token = "lan-shared-secret-token"
        val capturedRequestLine = AtomicReference("")
        val capturedHeaders = AtomicReference<Map<String, String>>(emptyMap())
        val capturedBody = AtomicReference("")
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))

        executor.submit {
            server.accept().use { socket ->
                val (line, headers, body) = readHttpRequest(socket)
                capturedRequestLine.set(line)
                capturedHeaders.set(headers)
                capturedBody.set(body)

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

                val ssePayload = listOf(
                    "data: ${json.encodeToString(deltaEnvelope)}\n\n",
                    "data: ${json.encodeToString(deviceToolEnvelope)}\n\n",
                    "data: [DONE]\n\n",
                ).joinToString("")
                writeHttpResponse(socket, "text/event-stream", ssePayload)
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
    fun pairedHostEndpointsPreservePathsHmacAndEncryptedRequestBodies() = runBlocking {
        val token = "lan-paired-body-token"
        val recorded = CopyOnWriteArrayList<Triple<String, Map<String, String>, JsonObject?>>()
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val expectedCalls = 16

        executor.submit {
            repeat(expectedCalls) {
                server.accept().use { socket ->
                    val (requestLine, headers, rawBody) = readHttpRequest(socket)
                    val decoded = rawBody.takeIf { it.isNotBlank() }?.let {
                        MobileCrypto.decryptJsonObject(json.decodeFromString<EncryptedEnvelope>(it), token)
                    }
                    recorded.add(Triple(requestLine, headers, decoded))

                    if (requestLine.contains("/tasks/events")) {
                        val checkpoint = MobileCrypto.encryptJsonObject(
                            buildJsonObject {
                                put("type", "task_stream_checkpoint")
                                put("conversationId", "conv-9")
                                put("cursor", 12)
                            },
                            token,
                        ).copy(type = "encrypted")
                        writeHttpResponse(socket, "text/event-stream", "data: ${json.encodeToString(checkpoint)}\n\ndata: [DONE]\n\n")
                    } else {
                        val replyObj = if (requestLine.contains("/agent/settings")) {
                            buildJsonObject { put("host_id", "host-9"); put("max_iterations", 16) }
                        } else {
                            buildJsonObject { put("ok", true); put("source_host_id", "host-1") }
                        }
                        val encryptedReply = json.encodeToString(
                            MobileCrypto.encryptJsonObject(replyObj, token).copy(type = "encrypted"),
                        )
                        writeHttpResponse(socket, "application/json", encryptedReply)
                    }
                }
            }
        }

        val client = HttpClient(CIO)
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))

            service.submitAgentDeviceToolResult(baseUrl, token, mapOf("session_id" to "conv-9", "request_id" to "dt-9", "output" to "ok"))
            service.goalControl(baseUrl, token, buildJsonObject { put("conversation_id", "conv-9"); put("action", "start") })
            service.auxiliaryCommand(baseUrl, token, "invoke", buildJsonObject { put("room_id", "room-1") })
            val taskEvents = service.streamAgentTaskEvents(baseUrl, token, "conv-9", afterCursor = 7L).toList()
            val settings = service.getAgentSettings(baseUrl, token)
            service.permissionConfiguration(baseUrl, token, buildJsonObject {
                put("operation", "read"); put("expected_host_id", "host-1"); put("conversation_id", "conv-1")
            })
            service.submissionFrontier(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1"); put("client_id", "client-1"); put("conversation_id", "conv-1")
            })
            service.conversationProgress(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1"); put("conversation_id", "conv-1")
            })
            service.observeRemoteConversation(baseUrl, token, buildJsonObject {
                put("expected_host_id", "host-1"); put("client_id", "client-1"); put("conversation_id", "conv-1")
            })
            service.generativeUi(baseUrl, token, "surfaces", buildJsonObject {
                put("expected_host_id", "host-1"); put("conversation_id", "conv-1")
            })
            service.generativeUi(baseUrl, token, "queued-input", buildJsonObject {
                put("expected_host_id", "host-1"); put("client_id", "client-1"); put("conversation_id", "conv-1")
            })
            service.readAssistantJournal(baseUrl, token, buildJsonObject {
                put("operation", "open_snapshot")
                put("at", buildJsonObject { put("identity", buildJsonObject { put("conversation_id", "conv-at") }) })
            })
            service.readAssistantJournal(baseUrl, token, buildJsonObject {
                put("operation", "record_chunk")
                put("snapshot", buildJsonObject {
                    put("at", buildJsonObject { put("identity", buildJsonObject { put("conversation_id", "conv-snap") }) })
                })
            })
            service.remoteSubmission(baseUrl, token, buildJsonObject {
                put("operation", "cancel")
                put("identity", buildJsonObject { put("conversation_id", "conv-sub") })
                put("request_digest", "a".repeat(64))
            })
            service.taskCancellationCommand(baseUrl, token, buildJsonObject {
                put("hostId", "host-1"); put("conversationId", "conv-task"); put("parentRunId", "run-1"); put("taskId", "task-42")
            }, receiptOnly = true)
            service.taskGuidanceCommand(baseUrl, token, buildJsonObject {
                put("hostId", "host-1"); put("conversationId", "conv-task"); put("parentRunId", "run-1")
                put("taskId", "task-42"); put("messageId", "msg-7"); put("message", "focus on tests")
            }, receiptOnly = false)

            assertEquals(expectedCalls, recorded.size)
            assertTrue(recorded[0].first.startsWith("POST /api/mobile/chat/conversations/conv-9/device-tool "))
            assertTrue(recorded[1].first.startsWith("POST /api/mobile/chat/conversations/conv-9/goal "))
            assertTrue(recorded[2].first.startsWith("POST /api/mobile/auxiliary/invoke "))
            assertTrue(recorded[3].first.startsWith("GET /api/mobile/chat/conversations/conv-9/tasks/events?afterCursor=7 "))
            assertTrue(recorded[4].first.startsWith("GET /api/mobile/agent/settings "))
            assertEquals(listOf(SseEvent.TaskStreamCheckpoint("conv-9", 12L)), taskEvents)
            assertEquals(VerifiedHostTransport.DirectWithoutTailcat, settings.verifiedTransport)

            assertTrue(recorded[5].first.startsWith("POST /api/mobile/chat/conversations/conv-1/configuration "))
            assertEquals(JsonPrimitive("read"), recorded[5].third?.get("operation"))
            assertTrue(recorded[6].first.startsWith("POST /api/mobile/chat/conversations/conv-1/configuration "))
            assertEquals(JsonPrimitive("client-1"), recorded[6].third?.get("client_id"))
            assertTrue(recorded[7].first.startsWith("POST /api/mobile/chat/conversations/conv-1/progress?expected_host_id=host-1&conversation_id=conv-1 "))
            assertEquals(JsonPrimitive("host-1"), recorded[7].third?.get("expected_host_id"))
            assertTrue(recorded[8].first.startsWith("POST /api/mobile/chat/conversations/conv-1/observe "))
            assertEquals(JsonPrimitive("client-1"), recorded[8].third?.get("client_id"))
            assertTrue(recorded[9].first.startsWith("POST /api/mobile/chat/conversations/conv-1/ui-surfaces "))
            assertEquals(JsonPrimitive("host-1"), recorded[9].third?.get("expected_host_id"))
            assertTrue(recorded[10].first.startsWith("POST /api/mobile/chat/conversations/conv-1/events "))
            assertEquals(JsonPrimitive("host-1"), recorded[10].third?.get("expected_host_id"))
            assertTrue(recorded[11].first.startsWith("POST /api/mobile/chat/conversations "))
            assertEquals(JsonPrimitive("conv-at"), recorded[11].third?.get("conversation_id"))
            assertTrue(recorded[12].first.startsWith("POST /api/mobile/chat/conversations "))
            assertEquals(JsonPrimitive("conv-snap"), recorded[12].third?.get("conversation_id"))
            assertTrue(recorded[13].first.startsWith("POST /api/mobile/chat/conversations/conv-sub "))
            assertEquals(JsonPrimitive("cancel"), recorded[13].third?.get("operation"))
            assertTrue(recorded[14].first.startsWith("POST /api/mobile/chat/conversations/conv-task/tasks/task-42/stop "))
            assertEquals(JsonPrimitive(true), recorded[14].third?.get("receipt_only"))
            assertTrue(recorded[15].first.startsWith("POST /api/mobile/chat/conversations/conv-task/messages "))
            assertEquals(JsonPrimitive("focus on tests"), recorded[15].third?.get("message"))
            assertEquals(JsonPrimitive(false), recorded[15].third?.get("receipt_only"))

            for ((line, headers, _) in recorded) {
                assertEquals("Missing aes-gcm-v1 on $line", "aes-gcm-v1", headers["x-nola-mobile-crypto"])
                assertNotNull("Missing HMAC proof on $line", headers["x-nola-mobile-auth"])
                assertFalse("Plaintext Bearer token must not be sent on $line", headers.containsKey("authorization"))
                if (line.startsWith("POST ")) {
                    assertTrue("Expected JSON Content-Type on $line", headers["content-type"]?.startsWith("application/json") == true)
                }
            }
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    // -------------------------------------------------------------------------
    // 2. Canonical PublicPartSemantics Decoding & Leakage Boundaries
    // -------------------------------------------------------------------------

    @Test
    fun publicPartSemanticsDecodesCoreOriginSharedFactsAndRejectsLeakage() {
        val root = Json.parseToJsonElement(SHARED_PROJECTION_JSON).jsonObject
        val projection = ConversationProjectionV1Decoder.decode(root)
        val assistant = CanonicalConversationProjector.project(projection)[1]
        val reasoning = assistant.canonicalParts[0] as ConversationProjectionV1.Part.Reasoning
        val facts = (assistant.canonicalParts[1] as ConversationProjectionV1.Part.Tool).cardFacts()

        assertEquals("Inspect evidence", reasoning.title)
        assertEquals("Read architecture", facts.title)
        assertEquals("Read the contract", facts.summary)
        assertEquals(Json.parseToJsonElement("""{"text":"Architecture contract"}"""), facts.result)
        assertEquals("Independent technical preview", facts.preview)
        assertEquals("builtin", facts.origin.kind)
        assertEquals("read", facts.artifacts.single().operation)
        assertEquals(1787443201000uL, facts.startedAtMs)
        assertEquals(1787443201010uL, facts.finishedAtMs)

        // Core origin kind and strict non-coercion checks.
        assertEquals(CapabilityOrigin(), PublicPartSemanticsDecoder.origin(JsonObject(emptyMap())))
        assertEquals(CapabilityOrigin("core"), PublicPartSemanticsDecoder.origin(Json.parseToJsonElement("""{"kind":"core"}""")))
        assertFails { PublicPartSemanticsDecoder.origin(Json.parseToJsonElement("""{"kind":null}""")) }
        assertFails { PublicPartSemanticsDecoder.origin(Json.parseToJsonElement("""{"kind":"builtin","provider_title":42}""")) }
        assertFails { PublicPartSemanticsDecoder.permission(Json.parseToJsonElement("""{"request_id":"r","permission_domain":"file","rationale":42}""")) }
        assertFails { PublicPartSemanticsDecoder.location(Json.parseToJsonElement("""{"section":true}""")) }
        for (badUnsigned in listOf("-1", "1.0", "1e3", "true", "18446744073709551616")) {
            assertFails { PublicPartSemanticsDecoder.unsigned(Json.parseToJsonElement(badUnsigned)) }
        }
        assertFails { PublicPartSemanticsDecoder.unsigned(JsonPrimitive("1")) }
        assertNull(ULong.MAX_VALUE.asLegacyLongOrNull())
        assertEquals(Long.MAX_VALUE, Long.MAX_VALUE.toULong().asLegacyLongOrNull())

        fun projected(parts: List<JsonElement>) = CanonicalConversationProjector.project(
            ConversationProjectionV1Decoder.decode(
                JsonObject(
                    root + ("messages" to JsonArray(
                        listOf(JsonObject(root.getValue("messages").jsonArray[1].jsonObject + ("parts" to JsonArray(parts)))),
                    )),
                ),
            ),
        ).single()

        val toolPart = root.getValue("messages").jsonArray[1].jsonObject.getValue("parts").jsonArray[1].jsonObject

        // Source, citation coordinates, and attachment semantics remain intact.
        val source = Json.parseToJsonElement(
            """{"kind":"source","part_id":"source-part","revision":7,"source_id":"source","title":"Evidence","uri":"memory://source","mime_type":"text/plain","snippet":"snippet","evidence":{"kind":"memory","usage":{"record_id":"record","component_id":"component","scope":"turn","namespace":{"kind":"custom","name":"workspace-not-kind"},"provenance_kind":"tool_result","source":"source-uri","title":"included title","observed_at":"observed","source_role":"tool","event_time":"event","range_start":"start","range_end":"end","requires_current_state_verification":true,"truncated":true,"supersedes":["older"],"usage_count":18446744073709551615,"last_used_at_epoch_ms":-9223372036854775808}}}""",
        )
        val citation = Json.parseToJsonElement(
            """{"kind":"citation","part_id":"part-coord","revision":7,"citation_id":"real-coord","source_id":"source","title":"Report","uri":"https://example.com/report","mime_type":"application/pdf","origin_tool_call_id":"call","location":{"page":18446744073709551615,"line_start":0,"line_end":18446744073709551615,"section":"结果🙂"},"page":18446744073709551615,"line_start":0,"line_end":18446744073709551615,"section":"结果🙂","range_start":0,"range_end":18446744073709551615}""",
        )
        val attachment = Json.parseToJsonElement(
            """{"kind":"attachment","part_id":"artifact-part","revision":7,"attachment_id":"attachment","name":"report.pdf","mime_type":"application/pdf","uri":"file:///tmp/report.pdf","operation":"write"}""",
        )
        val rich = projected(listOf(source, citation, attachment))
        val usage = rich.canonicalParts[0].sourceCardFacts()!!.evidence!!.usage
        assertEquals(MemoryNamespace("custom", "workspace-not-kind"), usage.namespace)
        assertEquals(ULong.MAX_VALUE, usage.usageCount)
        assertEquals(Long.MIN_VALUE, usage.lastUsedAtEpochMs)
        val citationFacts = rich.canonicalParts[1].sourceCardFacts()!!
        assertEquals(SourceLocation(ULong.MAX_VALUE, 0uL, ULong.MAX_VALUE, "结果🙂"), citationFacts.location)
        assertEquals(CitationRange(0uL, ULong.MAX_VALUE), citationFacts.range)
        assertEquals(listOf("source-part", "real-coord"), decodeAnswerSourcesFromMeta(rich.meta).map { it.id })
        assertEquals("write", (rich.canonicalParts[2] as ConversationProjectionV1.Part.Attachment).operation)

        // Tool full_output owner binding & private-field leakage rejection.
        val validTool = JsonObject(toolPart + ("full_output" to Json.parseToJsonElement("""{"message_id":"contract-assistant-1","part_id":"contract-assistant-1:tool:call-1","tool_call_id":"call-1"}""")))
        assertEquals(
            ToolOutputOwner("contract-assistant-1", "contract-assistant-1:tool:call-1", "call-1"),
            (projected(listOf(validTool)).canonicalParts.single() as ConversationProjectionV1.Part.Tool).cardFacts().fullOutput,
        )
        for (review in listOf("pending", "rejected", "interrupted")) {
            val hidden = Json.parseToJsonElement("""{"kind":"text","part_id":"text","revision":7,"markdown":"","is_final":true,"review":"$review"}""").jsonObject
            assertEquals("", (projected(listOf(hidden)).canonicalParts.single() as ConversationProjectionV1.Part.Text).disclosedMarkdown)
            assertFails { projected(listOf(JsonObject(hidden + ("markdown" to JsonPrimitive("private response"))))) }
        }
        for (privateField in listOf("full_output_ref", "raw_input", "grant")) {
            assertFails { projected(listOf(JsonObject(toolPart + (privateField to JsonPrimitive("private"))))) }
        }
        assertFails { projected(listOf(JsonObject(toolPart + ("full_output" to Json.parseToJsonElement("""{"message_id":"foreign","part_id":"contract-assistant-1:tool:call-1","tool_call_id":"call-1"}"""))))) }
        assertFails { PublicPartSemanticsDecoder.evidence(Json.parseToJsonElement("""{"kind":"memory","usage":{"record_id":"r","component_id":"c","body":"private memory"}}""")) }
        assertFails { PublicPartSemanticsDecoder.permission(Json.parseToJsonElement("""{"request_id":"r","permission_domain":"file","rationale":"why","lease":"private"}""")) }
    }

    // -------------------------------------------------------------------------
    // 3. Design Tokens, WCAG AA Contrast & 6-Locale Completeness
    // -------------------------------------------------------------------------

    @Test
    fun designTokensMatchThemeSchemesAndMeetWcagAaContrast() {
        val tokenFile = generateSequence(File(".").absoluteFile) { it.parentFile }
            .map { File(it, "shared/design-tokens/tokens.json") }
            .firstOrNull(File::isFile)
            ?: error("Could not locate shared/design-tokens/tokens.json from ${File(".").absolutePath}")
        val colorRoot = json.parseToJsonElement(tokenFile.readText()).jsonObject.getValue("color").jsonObject
        val light = colorRoot.getValue("light").jsonObject.mapValues { parseHexColor(it.value.jsonPrimitive.content) }
        val dark = colorRoot.getValue("dark").jsonObject.mapValues { parseHexColor(it.value.jsonPrimitive.content) }

        assertEquals(light.getValue("canvas"), NolaColors.bgLight)
        assertEquals(light.getValue("canvas"), LightColors.background)
        assertEquals(light.getValue("surface"), LightColors.surface)
        assertEquals(light.getValue("surfaceSubtle"), LightColors.surfaceContainer)
        assertEquals(light.getValue("ink"), LightColors.onBackground)
        assertEquals(light.getValue("inkSecondary"), LightColors.onSurfaceVariant)
        assertEquals(light.getValue("accent"), LightColors.primary)
        assertEquals(light.getValue("onAccent"), LightColors.onPrimary)
        assertEquals(light.getValue("danger"), LightColors.error)

        assertEquals(dark.getValue("canvas"), NolaColors.bgDark)
        assertEquals(dark.getValue("canvas"), DarkColors.background)
        assertEquals(dark.getValue("surface"), DarkColors.surface)
        assertEquals(dark.getValue("surfaceSubtle"), DarkColors.surfaceContainer)
        assertEquals(dark.getValue("ink"), DarkColors.onBackground)
        assertEquals(dark.getValue("inkSecondary"), DarkColors.onSurfaceVariant)
        assertEquals(dark.getValue("accent"), DarkColors.primary)
        assertEquals(dark.getValue("onAccent"), DarkColors.onPrimary)
        assertEquals(dark.getValue("danger"), DarkColors.error)

        assertNotEquals(Color(0xFF000000), NolaColors.bgDark)
        assertNotEquals(Color(0xFFFFFFFF), NolaColors.bgLight)

        listOf(LightColors, DarkColors).forEach { scheme ->
            val pairs = listOf(
                scheme.onBackground to scheme.background,
                scheme.onSurface to scheme.surface,
                scheme.onSurfaceVariant to scheme.surfaceContainer,
                scheme.onPrimary to scheme.primary,
                scheme.onPrimaryContainer to scheme.primaryContainer,
                scheme.primary to scheme.surface,
                scheme.error to scheme.surface,
                scheme.onSurfaceVariant to compositeOver(scheme.surfaceVariant.copy(alpha = 0.50f), scheme.surface),
                scheme.primary to compositeOver(scheme.primary.copy(alpha = 0.12f), scheme.surface),
                scheme.error to compositeOver(scheme.error.copy(alpha = 0.12f), scheme.surface),
            )
            pairs.forEach { (fg, bg) ->
                val ratio = contrastRatio(fg, bg)
                assertTrue("Semantic text contrast ${"%.2f".format(ratio)} is below 4.5:1", ratio >= 4.5)
            }
        }
    }

    @Test
    fun allSixLocalesHaveCompleteStringAndPluralParityWithMatchingPlaceholders() {
        val resRoot = listOf(File("src/main/res"), File("apps/android/app/src/main/res"))
            .firstOrNull(File::isDirectory)
            ?: error("Could not locate Android res directory from ${File(".").absolutePath}")

        val supportedLocales = listOf("values-de", "values-es", "values-fr", "values-ja", "values-ru", "values-zh")
        val base = parseLocaleDirectory(File(resRoot, "values"))
        assertTrue("Expected base XML string resources", base.strings.isNotEmpty())
        assertTrue("Expected base XML plurals resources", base.plurals.isNotEmpty())

        val requiredStrings = base.strings.filterValues { it.translatable }.keys
        val requiredPlurals = base.plurals.keys

        for (locale in supportedLocales) {
            val localeDir = File(resRoot, locale)
            assertTrue("Missing localized resource directory: $locale", localeDir.isDirectory)
            val localized = parseLocaleDirectory(localeDir)

            assertEquals("$locale XML file set mismatch", base.fileNames, localized.fileNames)
            assertTrue("$locale missing strings: ${(requiredStrings - localized.strings.keys).sorted()}", (requiredStrings - localized.strings.keys).isEmpty())
            assertTrue("$locale stale strings: ${(localized.strings.keys - base.strings.keys).sorted()}", (localized.strings.keys - base.strings.keys).isEmpty())
            assertTrue("$locale missing plurals: ${(requiredPlurals - localized.plurals.keys).sorted()}", (requiredPlurals - localized.plurals.keys).isEmpty())
            assertTrue("$locale stale plurals: ${(localized.plurals.keys - requiredPlurals).sorted()}", (localized.plurals.keys - requiredPlurals).isEmpty())

            for (key in requiredStrings) {
                val baseEntry = base.strings.getValue(key)
                val locEntry = localized.strings.getValue(key)
                assertTrue("$locale/$key has empty text", locEntry.value.isNotBlank())
                if (baseEntry.formatted && locEntry.formatted) {
                    assertEquals(
                        "$locale/$key placeholder mismatch",
                        extractPlaceholders(baseEntry.value),
                        extractPlaceholders(locEntry.value),
                    )
                }
            }
            for (key in requiredPlurals) {
                val baseSignatures = base.plurals.getValue(key).values.map(::extractPlaceholders).toSet()
                val locItems = localized.plurals.getValue(key)
                assertTrue("$locale/$key has no plural <item> entries", locItems.isNotEmpty())
                for ((qty, text) in locItems) {
                    assertTrue("$locale/$key[$qty] has empty text", text.isNotBlank())
                    assertTrue("$locale/$key[$qty] placeholders mismatch", extractPlaceholders(text) in baseSignatures)
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    private fun readHttpRequest(socket: java.net.Socket): Triple<String, Map<String, String>, String> {
        val input = BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
        val requestLine = input.readLine().orEmpty()
        val headers = mutableMapOf<String, String>()
        while (true) {
            val line = input.readLine() ?: break
            if (line.isEmpty()) break
            val sep = line.indexOf(':')
            if (sep > 0) headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
        }
        val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
        val chars = CharArray(bodyLength)
        var offset = 0
        while (offset < bodyLength) {
            val read = input.read(chars, offset, bodyLength - offset)
            if (read < 0) break
            offset += read
        }
        return Triple(requestLine, headers, String(chars, 0, offset))
    }

    private fun writeHttpResponse(socket: java.net.Socket, contentType: String, body: String) {
        val bytes = body.toByteArray(Charsets.UTF_8)
        val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
        output.write(
            "HTTP/1.1 200 OK\r\n" +
                "Content-Type: $contentType\r\n" +
                "Connection: close\r\n" +
                "Content-Length: ${bytes.size}\r\n\r\n",
        )
        output.write(body)
        output.flush()
    }

    private fun parseHexColor(hex: String): Color {
        val cleaned = hex.trim().removePrefix("#")
        require(cleaned.length == 6) { "Expected 6-digit hex color, got: $hex" }
        return Color(0xFF000000L or cleaned.toLong(16))
    }

    private fun compositeOver(fg: Color, bg: Color): Color {
        val a = fg.alpha
        val inv = 1f - a
        return Color(fg.red * a + bg.red * inv, fg.green * a + bg.green * inv, fg.blue * a + bg.blue * inv, 1f)
    }

    private fun contrastRatio(first: Color, second: Color): Double {
        fun linear(v: Float) = v.toDouble().let { if (it <= 0.04045) it / 12.92 else ((it + 0.055) / 1.055).pow(2.4) }
        fun lum(c: Color) = linear(c.red) * 0.2126 + linear(c.green) * 0.7152 + linear(c.blue) * 0.0722
        val a = lum(first)
        val b = lum(second)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    private data class StringEntry(val translatable: Boolean, val formatted: Boolean, val value: String)
    private data class LocaleResources(
        val fileNames: Set<String>,
        val strings: Map<String, StringEntry>,
        val plurals: Map<String, Map<String, String>>,
    )

    private fun parseLocaleDirectory(directory: File): LocaleResources {
        val nonTranslatable = setOf("colors.xml", "themes.xml")
        val factory = DocumentBuilderFactory.newInstance()
        val xmlFiles = directory.listFiles().orEmpty()
            .filter { it.isFile && it.extension == "xml" && it.name !in nonTranslatable }
            .sortedBy { it.name }
        val strings = linkedMapOf<String, StringEntry>()
        val plurals = linkedMapOf<String, Map<String, String>>()
        for (file in xmlFiles) {
            val doc = factory.newDocumentBuilder().parse(file)
            val sNodes = doc.getElementsByTagName("string")
            for (i in 0 until sNodes.length) {
                val el = sNodes.item(i)
                val attrs = el.attributes ?: continue
                val name = attrs.getNamedItem("name")?.nodeValue?.takeIf(String::isNotBlank) ?: continue
                assertTrue("Duplicate <string name=\"$name\"> in ${directory.name}/${file.name}", name !in strings)
                strings[name] = StringEntry(
                    translatable = attrs.getNamedItem("translatable")?.nodeValue != "false",
                    formatted = attrs.getNamedItem("formatted")?.nodeValue != "false",
                    value = el.textContent.orEmpty(),
                )
            }
            val pNodes = doc.getElementsByTagName("plurals")
            for (i in 0 until pNodes.length) {
                val el = pNodes.item(i)
                val name = el.attributes?.getNamedItem("name")?.nodeValue?.takeIf(String::isNotBlank) ?: continue
                val items = linkedMapOf<String, String>()
                for (j in 0 until el.childNodes.length) {
                    val child = el.childNodes.item(j)
                    if (child.nodeName == "item") {
                        val qty = child.attributes?.getNamedItem("quantity")?.nodeValue ?: continue
                        items[qty] = child.textContent.orEmpty()
                    }
                }
                assertTrue("Duplicate <plurals name=\"$name\"> in ${directory.name}/${file.name}", name !in plurals)
                plurals[name] = items
            }
        }
        return LocaleResources(xmlFiles.mapTo(mutableSetOf()) { it.name }, strings, plurals)
    }

    private fun extractPlaceholders(text: String): List<Pair<Int, Char>> {
        val regex = Regex("""%(?:(\d+)\$)?([-#+ 0,(]*\d*(?:\.\d+)?[bBhHsScCdoxXeEfgGaAtT%])""")
        var nextPos = 1
        return buildList {
            for (m in regex.findAll(text)) {
                val tail = m.groupValues[2]
                if (tail == "%") continue
                val pos = m.groupValues[1].toIntOrNull() ?: nextPos++
                add(pos to tail.last())
            }
        }.sortedWith(compareBy({ it.first }, { it.second }))
    }

    private companion object {
        private val SHARED_PROJECTION_JSON = """
            {
              "contract_version": 1,
              "conversation_id": "contract-conversation-1",
              "revision": 7,
              "title": "Host projection contract",
              "messages": [
                {
                  "message_id": "contract-user-1",
                  "role": "user",
                  "state": "completed",
                  "revision": 7,
                  "created_at_ms": 1787443200000,
                  "parts": [
                    {
                      "kind": "text",
                      "part_id": "contract-user-1:text",
                      "revision": 7,
                      "markdown": "Explain the contract.",
                      "is_final": true
                    }
                  ]
                },
                {
                  "message_id": "contract-assistant-1",
                  "role": "assistant",
                  "state": "completed",
                  "revision": 7,
                  "created_at_ms": 1787443201000,
                  "parts": [
                    {
                      "kind": "reasoning",
                      "part_id": "contract-assistant-1:reasoning",
                      "revision": 7,
                      "state": "completed",
                      "title": "Inspect evidence",
                      "summary": "Inspect the shared projection."
                    },
                    {
                      "kind": "tool",
                      "execution_source": { "kind": "model" },
                      "part_id": "contract-assistant-1:tool:call-1",
                      "revision": 7,
                      "tool_call_id": "call-1",
                      "capability_id": "file.read",
                      "state": "completed",
                      "display_key": "tool.file.read",
                      "title": "Read architecture",
                      "summary": "Read the contract",
                      "origin": { "kind": "builtin" },
                      "permission": null,
                      "output_preview": "Independent technical preview",
                      "full_output": null,
                      "artifacts": [
                        {
                          "uri": "file:///workspace/ARCHITECTURE.md",
                          "name": "ARCHITECTURE.md",
                          "mime_type": "text/markdown",
                          "operation": "read"
                        }
                      ],
                      "started_at_ms": 1787443201000,
                      "finished_at_ms": 1787443201010,
                      "input": { "path": "ARCHITECTURE.md" },
                      "output": { "text": "Architecture contract" },
                      "affected_resources": [
                        {
                          "resource_type": "text/markdown",
                          "uri": "file:///workspace/ARCHITECTURE.md",
                          "operation": "read"
                        }
                      ]
                    },
                    {
                      "kind": "text",
                      "part_id": "contract-assistant-1:text",
                      "revision": 7,
                      "markdown": "The projection is the durable UI source.",
                      "is_final": true
                    }
                  ]
                }
              ],
              "active_run": null,
              "surface": {
                "revision": 7,
                "committed_message_ids": ["contract-user-1", "contract-assistant-1"],
                "live_message_id": null,
                "current_activity_part_id": null,
                "archived_activity_part_ids": [
                  "contract-assistant-1:reasoning",
                  "contract-assistant-1:tool:call-1"
                ],
                "show_assistant_placeholder": false
              },
              "context_usage": {
                "used_tokens": 4096,
                "context_window_tokens": 65536,
                "prompt_token_limit": 57344,
                "estimated": false,
                "measured_at_revision": 7
              },
              "execution_ownership": { "kind": "user" }
            }
        """.trimIndent()
    }
}
