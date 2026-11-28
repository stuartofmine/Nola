// SPDX-License-Identifier: Apache-2.0
package com.nola

import com.nola.core.AgentCoreNative
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.ApplicationSubmissionJournal
import com.nola.data.remote.api.FrozenRemoteSubmission
import com.nola.data.remote.api.RemoteRunKey
import com.nola.data.remote.api.RemoteSubmissionIdentity
import com.nola.data.remote.api.WebHostApiServiceImpl
import com.nola.data.remote.api.WebHostHttpClientFactory
import com.nola.data.remote.api.buildBoundAgentStreamPayload
import com.nola.data.remote.api.buildWebHostAgentRequestBody
import com.nola.data.remote.api.remoteResumeRequest
import com.nola.data.remote.dto.VerifiedHostTransport
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.data.repository.ModelConnectionRepository
import com.nola.domain.model.ModelAttachment
import com.nola.domain.model.ModelInput
import com.nola.domain.model.providerMessages
import com.nola.ui.screen.home.TodayPendingResponseReader
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.engine.cio.CIO
import io.ktor.client.request.get
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.File
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidAuditRegressionsTest {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    @Test
    fun todayPendingResponseReaderIgnoresUnknownKeysAndDecodesCorePayloadWithOptionalFields() {
        val rawCorePayload = buildJsonObject {
            put("date", "2026-11-27")
            put("sessions_today", buildJsonArray {
                add(buildJsonObject {
                    put("id", "conv-1")
                    put("title", "Morning standup")
                    put("updated_at", "2026-11-27T09:00:00Z")
                })
            })
            put("active_goals", buildJsonArray {})
            put("completed_goals_today", buildJsonArray {})
            put("facts", buildJsonArray { add(JsonPrimitive("1 conversation(s) active today")) })
            put("attention_items", buildJsonArray {
                add(buildJsonObject {
                    put("conversation_id", "conv-1")
                    put("title", "Morning standup")
                    put("reason", "waiting_for_user_input")
                })
            })
            put("conversations", buildJsonArray {
                add(buildJsonObject {
                    put("conversationId", "conv-1")
                    put("id", "conv-1")
                    put("title", "Morning standup")
                    put("isPinned", false)
                    put("revision", 3)
                    put("updated_at_ms", 1787443200000L)
                    put("executionBackend", "nola-v2")
                })
                add(buildJsonObject {
                    put("conversation_id", "conv-2")
                    put("title", "Needs approval")
                    put("revision", 5)
                    put("updated_at_ms", 1787443300000L)
                    put("needs_response", true)
                    put("active_run", buildJsonObject {
                        put("run_id", "run-2")
                        put("assistant_message_id", "asst-2")
                    })
                    put("open_waits", buildJsonArray { add(buildJsonObject { put("id", "wait-1") }) })
                })
            })
            put("recent_conversations", buildJsonArray {
                add(buildJsonObject {
                    put("conversationId", "conv-1")
                    put("title", "Morning standup")
                    put("revision", 3)
                    put("updated_at_ms", 1787443200000L)
                })
            })
            put("unrepresented_expert_contexts", buildJsonArray {})
            put("needs_attention", true)
            put("has_more_conversations", false)
            put("next_conversation_offset", JsonNull)
            put("queued_instances", buildJsonArray {})
            put("has_more_queued_instances", false)
        }

        val reader = TodayPendingResponseReader { query ->
            assertEquals(JsonPrimitive("load_today_facts"), query["type"])
            rawCorePayload
        }

        val snapshot = reader.read()
        assertTrue(snapshot.needsAttention)
        assertEquals(2, snapshot.conversations.size)
        assertEquals("conv-1", snapshot.conversations[0].conversationId)
        assertEquals("Morning standup", snapshot.conversations[0].title)
        assertFalse(snapshot.conversations[0].needsResponse)
        assertNull(snapshot.conversations[0].activeRun)
        assertTrue(snapshot.conversations[0].openWaits.isEmpty())
        assertTrue(snapshot.conversations[0].expertContexts.isEmpty())

        assertEquals("conv-2", snapshot.conversations[1].conversationId)
        assertTrue(snapshot.conversations[1].needsResponse)
        assertEquals("run-2", snapshot.conversations[1].activeRun?.runId)
        assertEquals(1, snapshot.conversations[1].openWaits.size)
        assertEquals(1, snapshot.recentConversations.size)
        assertNull(snapshot.nextConversationOffset)
    }

    @Test
    fun streamBoundAgentRequestPreservesMultimodalAttachmentsModelIntentProfileIdAndSubmissionMetadata() = runBlocking {
        val token = "bound-agent-stream-token"
        val messages = ModelInput(
            messages = listOf(mapOf("role" to "user", "content" to "Describe this diagram")),
            attachments = listOf(ModelAttachment(name = "diagram.png", mimeType = "image/png", base64Data = "iVBORw0KGgo=")),
        ).providerMessages()

        val identity = RemoteSubmissionIdentity(
            sourceHostId = "host-alpha",
            clientId = "android-client-1",
            conversationId = "conv-mm-1",
            submissionId = "sub-mm-1",
            userMessageId = "ui:sub-mm-1:user",
            assistantMessageId = "sub-mm-1:assistant",
        )
        val baseBody = buildWebHostAgentRequestBody(
            messages = messages,
            conversationId = identity.conversationId,
            allowLocalDevTools = false,
            maxIterations = 12,
            timezone = "Asia/Shanghai",
            language = "zh-CN",
            agentMode = true,
            permissionProfile = AndroidAgentPermissionMode.AllowAll,
            responseStyle = "concise",
            customSystemPrompt = "Be precise.",
            nativeAppHostId = "desktop-1",
            expectedHostId = identity.sourceHostId,
            clientId = identity.clientId,
            executionBackend = "nola",
            externalSessionId = "",
            submissionId = identity.submissionId,
            queuedUserCommandId = identity.submissionId,
        )
        val toolPlanRef = buildJsonObject {
            put("plan_id", "plan-42")
            put("digest", "b".repeat(64))
        }
        val modelIntent = buildJsonObject {
            put("profile_id", "profile-claude-4")
            put("expected_revision", 7)
            put("tier", "balanced")
            put("overrides", buildJsonObject {})
        }
        val frozenRequest = JsonObject(
            MobileCrypto.payloadJsonObject(baseBody) + mapOf(
                "model_intent" to modelIntent,
                "tool_plan_ref" to toolPlanRef,
                "external_session_id" to JsonPrimitive("ext-sess-9"),
            ),
        )
        val submission = FrozenRemoteSubmission(
            identity = identity,
            requestDigest = "a".repeat(64),
            request = frozenRequest,
        )

        // Unit check on payload builder directly
        val built = buildBoundAgentStreamPayload(submission)
        assertEquals(JsonPrimitive("Describe this diagram"), built["content"])
        assertEquals(JsonPrimitive("profile-claude-4"), built["modelId"])
        assertEquals(modelIntent, built["model_intent"])
        assertEquals(JsonPrimitive(identity.submissionId), built["submission_id"])
        assertEquals(JsonPrimitive(identity.userMessageId), built["user_message_id"])
        assertEquals(JsonPrimitive(identity.assistantMessageId), built["assistant_message_id"])
        assertEquals(JsonPrimitive(identity.submissionId), built["queued_user_command_id"])
        assertEquals(toolPlanRef, built["tool_plan_ref"])
        assertEquals(JsonPrimitive("Be precise."), built["custom_system_prompt"])
        assertEquals(JsonPrimitive("concise"), built["response_style"])
        assertEquals(JsonPrimitive(identity.sourceHostId), built["expected_host_id"])
        assertEquals(JsonPrimitive(identity.clientId), built["client_id"])
        assertEquals(JsonPrimitive("nola"), built["execution_backend"])
        assertEquals(JsonPrimitive("ext-sess-9"), built["external_session_id"])

        val builtMessages = built["messages"] as JsonArray
        assertEquals(1, builtMessages.size)
        val builtAttachments = built["attachments"] as JsonArray
        assertEquals(1, builtAttachments.size)
        assertEquals(
            JsonPrimitive("data:image/png;base64,iVBORw0KGgo="),
            builtAttachments[0].jsonObject["dataUrl"],
        )
        assertEquals(JsonPrimitive("image/png"), builtAttachments[0].jsonObject["mimeType"])

        // End-to-end encrypted wire verification over loopback HTTP
        val capturedLine = AtomicReference("")
        val capturedDecoded = AtomicReference<JsonObject?>(null)
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        executor.submit {
            server.accept().use { socket ->
                val (line, _, rawBody) = readHttpRequest(socket)
                capturedLine.set(line)
                capturedDecoded.set(
                    MobileCrypto.decryptJsonObject(json.decodeFromString<EncryptedEnvelope>(rawBody), token),
                )
                val delta = MobileCrypto.encryptJsonObject(
                    buildJsonObject {
                        put("type", "text_delta")
                        put("delta", "Image analyzed")
                    },
                    token,
                ).copy(type = "encrypted")
                writeHttpResponse(socket, "text/event-stream", "data: ${json.encodeToString(delta)}\n\ndata: [DONE]\n\n")
            }
        }

        val client = HttpClient(CIO)
        try {
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
            val events = service.streamBoundAgentRequest(
                hostBaseUrl = "http://127.0.0.1:${server.localPort}",
                token = token,
                submission = submission,
            ).toList()

            assertTrue(capturedLine.get().startsWith("POST /api/mobile/chat/conversations/conv-mm-1/messages "))
            assertEquals(listOf(SseEvent.V2TextDelta("Image analyzed")), events)
            val wire = requireNotNull(capturedDecoded.get())
            assertEquals(JsonPrimitive("Describe this diagram"), wire["content"])
            assertEquals(JsonPrimitive("profile-claude-4"), wire["modelId"])
            assertEquals(modelIntent, wire["model_intent"])
            assertEquals(toolPlanRef, wire["tool_plan_ref"])
            assertEquals(JsonPrimitive("Be precise."), wire["custom_system_prompt"])
            assertEquals(JsonPrimitive("concise"), wire["response_style"])
            assertEquals(JsonPrimitive("ext-sess-9"), wire["external_session_id"])
            assertEquals(1, wire["attachments"]?.jsonArray?.size)
            assertEquals(1, wire["messages"]?.jsonArray?.size)
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun resumeRemoteConversationObservesViaGetInsteadOfPostingEmptyPromptToMessages() = runBlocking {
        val token = "resume-observe-token"
        val recordedRequests = CopyOnWriteArrayList<Triple<String, Map<String, String>, String>>()
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        executor.submit {
            // Call 1: JSON observe response (standard v2 Host behavior)
            server.accept().use { socket ->
                val req = readHttpRequest(socket)
                recordedRequests.add(req)
                val encryptedObs = json.encodeToString(
                    MobileCrypto.encryptJsonObject(
                        buildJsonObject {
                            put("source_host_id", "host-1")
                            put("conversation_id", "conv-live")
                            put("client_id", "client-1")
                            put("result", buildJsonObject { put("status", "running") })
                        },
                        token,
                    ).copy(type = "encrypted"),
                )
                writeHttpResponse(socket, "application/json", encryptedObs)
            }
            // Call 2: SSE observe stream response
            server.accept().use { socket ->
                val req = readHttpRequest(socket)
                recordedRequests.add(req)
                val delta = MobileCrypto.encryptJsonObject(
                    buildJsonObject {
                        put("type", "text_delta")
                        put("delta", "resumed chunk")
                    },
                    token,
                ).copy(type = "encrypted")
                writeHttpResponse(socket, "text/event-stream", "data: ${json.encodeToString(delta)}\n\ndata: [DONE]\n\n")
            }
        }

        val client = HttpClient(CIO)
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
            val request = remoteResumeRequest(
                hostId = "host-1",
                clientId = "client-1",
                key = RemoteRunKey("conv-live", "run-77", "asst-77"),
            )

            val jsonModeEvents = service.resumeRemoteConversation(baseUrl, token, request).toList()
            val sseModeEvents = service.resumeRemoteConversation(baseUrl, token, request).toList()

            assertTrue(jsonModeEvents.isEmpty())
            assertEquals(listOf(SseEvent.V2TextDelta("resumed chunk")), sseModeEvents)
            assertEquals(2, recordedRequests.size)
            for ((line, headers, body) in recordedRequests) {
                assertTrue("Expected GET /observe, got: $line", line.startsWith("GET /api/mobile/chat/conversations/conv-live/observe?"))
                assertFalse("Must never POST to /messages", line.contains("/messages"))
                assertTrue(line.contains("expected_host_id=host-1"))
                assertTrue(line.contains("client_id=client-1"))
                assertTrue(line.contains("expected_run_id=run-77"))
                assertEquals("", body)
                assertEquals("aes-gcm-v1", headers["x-nola-mobile-crypto"])
                assertNotNull(headers["x-nola-mobile-auth"])
            }
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun expertMessageCommandPreservesOperationAndRequestKeys() = runBlocking {
        val token = "expert-op-token"
        val capturedBodies = CopyOnWriteArrayList<JsonObject>()
        val capturedLines = CopyOnWriteArrayList<String>()
        val executor = Executors.newSingleThreadExecutor()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val operations = listOf("parent", "admit", "manifest")
        executor.submit {
            repeat(operations.size) {
                server.accept().use { socket ->
                    val (line, _, rawBody) = readHttpRequest(socket)
                    capturedLines.add(line)
                    capturedBodies.add(
                        MobileCrypto.decryptJsonObject(json.decodeFromString<EncryptedEnvelope>(rawBody), token),
                    )
                    val reply = json.encodeToString(
                        MobileCrypto.encryptJsonObject(
                            buildJsonObject {
                                put("ok", true)
                                put("host_id", "host-1")
                                put("client_id", "client-1")
                            },
                            token,
                        ).copy(type = "encrypted"),
                    )
                    writeHttpResponse(socket, "application/json", reply)
                }
            }
        }

        val client = HttpClient(CIO)
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))
            for (op in operations) {
                service.expertMessageCommand(
                    hostBaseUrl = baseUrl,
                    token = token,
                    operation = op,
                    request = buildJsonObject {
                        put("expected_host_id", "host-1")
                        put("client_id", "client-1")
                        put("conversation_id", "conv-room-1")
                    },
                )
            }

            assertEquals(operations.size, capturedBodies.size)
            for (i in operations.indices) {
                assertTrue(capturedLines[i].startsWith("POST /api/mobile/agent-rooms "))
                assertEquals(JsonPrimitive(operations[i]), capturedBodies[i]["operation"])
                assertEquals(JsonPrimitive(operations[i]), capturedBodies[i]["type"])
                assertEquals(JsonPrimitive("host-1"), capturedBodies[i]["expected_host_id"])
                assertEquals(JsonPrimitive("client-1"), capturedBodies[i]["client_id"])
                assertEquals(JsonPrimitive("conv-room-1"), capturedBodies[i]["conversation_id"])
            }
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun liveNolaWebServerExercisesWebHostApiServiceEndToEnd() = runBlocking {
        val repoRoot = locateRepoRoot()
        val nolaBin = File(repoRoot, "target/debug/nola")
        val webRoot = File(repoRoot, "apps/web")
        assertTrue("Expected compiled nola binary at ${nolaBin.absolutePath}", nolaBin.isFile)
        assertTrue("Expected web root at ${webRoot.absolutePath}", webRoot.isDirectory)

        val tempRoot = createPrivateTempDir("nola-android-live-web-e2e-")
        val homeDir = File(tempRoot, "home").apply { mkdirs() }
        val workDir = File(tempRoot, "workspace").apply { mkdirs() }
        val dbFile = File(tempRoot, "nola.db")
        val liveToken = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"

        // Loopback fake OpenAI-compatible upstream LLM to verify real nola web forwarding
        val capturedUpstreamBody = AtomicReference<JsonObject?>(null)
        val upstreamExecutor = Executors.newSingleThreadExecutor()
        val upstreamServer = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        upstreamExecutor.submit {
            while (!upstreamServer.isClosed) {
                val socket = runCatching { upstreamServer.accept() }.getOrNull() ?: break
                socket.use { sock ->
                    val (reqLine, _, rawBody) = readHttpRequest(sock)
                    if (reqLine.startsWith("POST ") && reqLine.contains("/chat/completions")) {
                        val parsed = runCatching { json.parseToJsonElement(rawBody).jsonObject }.getOrNull()
                        capturedUpstreamBody.set(parsed)
                        val sseBody = buildString {
                            append(
                                """data: {"id":"chatcmpl-live-e2e","object":"chat.completion.chunk","created":1787443200,"model":"profile-live-vision","choices":[{"index":0,"delta":{"role":"assistant","content":"Live diagram analyzed OK"},"finish_reason":null}]}""",
                            )
                            append("\n\n")
                            append(
                                """data: {"id":"chatcmpl-live-e2e","object":"chat.completion.chunk","created":1787443200,"model":"profile-live-vision","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}""",
                            )
                            append("\n\n")
                            append("data: [DONE]\n\n")
                        }
                        writeHttpResponse(sock, "text/event-stream", sseBody)
                    } else {
                        writeHttpResponse(sock, "application/json", """{"data":[{"id":"profile-live-vision"}]}""")
                    }
                }
            }
        }

        val configFile = File(tempRoot, "nola-config.json")
        configFile.writeText(
            """
            {
              "llm": {
                "default_model": {
                  "connection": "mock_upstream",
                  "model": "default-fallback-model"
                },
                "connections": [
                  {
                    "id": "mock_upstream",
                    "endpoint": "http://127.0.0.1:${upstreamServer.localPort}/v1",
                    "protocol": "chat_completions",
                    "api_key": "mock-upstream-key",
                    "models": [
                      { "id": "default-fallback-model" },
                      {
                        "id": "profile-live-vision",
                        "capabilities": { "image_input": true }
                      }
                    ]
                  }
                ]
              },
              "network_policy": "allow",
              "max_iterations": 4
            }
            """.trimIndent(),
        )

        var webProc: Process? = null
        val client = HttpClient(CIO)
        try {
            runNolaCli(
                nolaBin = nolaBin,
                homeDir = homeDir,
                workDir = workDir,
                args = listOf("--db", dbFile.absolutePath, "--project", workDir.absolutePath, "config", "apply", configFile.absolutePath),
            )

            val webPort = findFreeLoopbackPort()
            val baseUrl = "http://127.0.0.1:$webPort"
            val webLog = File(tempRoot, "nola-web.log")
            val pb = ProcessBuilder(
                nolaBin.absolutePath,
                "--db",
                dbFile.absolutePath,
                "--project",
                workDir.absolutePath,
                "web",
                "--host",
                "127.0.0.1",
                "--port",
                webPort.toString(),
                "--web-root",
                webRoot.absolutePath,
            )
            pb.directory(workDir)
            pb.redirectErrorStream(true)
            pb.redirectOutput(webLog)
            pb.environment().apply {
                put("HOME", homeDir.absolutePath)
                put("NOLA_WEB_ACCESS_TOKEN", liveToken)
                put("NO_PROXY", "127.0.0.1,localhost")
                put("no_proxy", "127.0.0.1,localhost")
            }
            webProc = pb.start()

            // 1. Health & session check against live nola web
            val health = waitForLiveHealth(client, baseUrl, webProc, webLog)
            assertEquals(JsonPrimitive(true), health["ok"])
            assertEquals(JsonPrimitive("2.0.0"), health["version"])
            assertEquals(JsonPrimitive("nola-v2"), health["service"])
            val hostId = requireNotNull(health["host"]?.jsonObject?.get("id")?.jsonPrimitive?.contentOrNull)
            assertTrue("Expected non-blank host.id from /api/health", hostId.isNotBlank())

            val sessionResp = client.get("$baseUrl/api/web/session")
            assertEquals("Expected /api/web/session 200 OK", 200, sessionResp.status.value)

            val service = WebHostApiServiceImpl(client, SseClient(client), WebHostHttpClientFactory(client))

            // Authenticated mobile settings check (real HMAC + AES-GCM mobile crypto)
            val settings = service.getAgentSettings(baseUrl, liveToken)
            assertEquals(VerifiedHostTransport.DirectWithoutTailcat, settings.verifiedTransport)

            // 2. Create real conversation + streamBoundAgentRequest with multimodal attachment + model_intent.profile_id
            val conversationId = service.v2CreateConversation(baseUrl, liveToken, "Live WebHost Multimodal E2E")
            assertTrue(conversationId.isNotBlank())

            val clientId = "android-live-e2e-client"
            val identity = RemoteSubmissionIdentity(
                sourceHostId = hostId,
                clientId = clientId,
                conversationId = conversationId,
                submissionId = "sub-live-mm-1",
                userMessageId = "ui:sub-live-mm-1:user",
                assistantMessageId = "sub-live-mm-1:assistant",
            )
            val messages = ModelInput(
                messages = listOf(mapOf("role" to "user", "content" to "Describe this diagram")),
                attachments = listOf(
                    ModelAttachment(name = "diagram.png", mimeType = "image/png", base64Data = "iVBORw0KGgo="),
                ),
            ).providerMessages()
            val baseBody = buildWebHostAgentRequestBody(
                messages = messages,
                conversationId = conversationId,
                allowLocalDevTools = false,
                maxIterations = 4,
                timezone = "UTC",
                language = "en-US",
                agentMode = true,
                permissionProfile = AndroidAgentPermissionMode.AllowAll,
                responseStyle = "concise",
                customSystemPrompt = "",
                nativeAppHostId = "",
                expectedHostId = hostId,
                clientId = clientId,
                executionBackend = "nola",
                externalSessionId = "",
                submissionId = identity.submissionId,
                queuedUserCommandId = identity.submissionId,
            )
            val modelIntent = buildJsonObject {
                put("profile_id", "profile-live-vision")
                put("expected_revision", 1)
                put("tier", "balanced")
                put("overrides", buildJsonObject {})
            }
            val frozenSubmission = FrozenRemoteSubmission(
                identity = identity,
                requestDigest = "a".repeat(64),
                request = JsonObject(MobileCrypto.payloadJsonObject(baseBody) + ("model_intent" to modelIntent)),
            )

            val streamEvents = service.streamBoundAgentRequest(baseUrl, liveToken, frozenSubmission).toList()
            val streamedText = streamEvents.filterIsInstance<SseEvent.V2TextDelta>().joinToString("") { it.delta }
            assertTrue(
                "Expected live encrypted SSE reply from mock upstream, got events=$streamEvents",
                streamedText.contains("Live diagram analyzed OK"),
            )

            val upstreamReq = requireNotNull(capturedUpstreamBody.get()) {
                "Expected live nola web to call loopback upstream LLM"
            }
            assertEquals(
                "Expected model_intent.profile_id to override default model on live nola web",
                JsonPrimitive("profile-live-vision"),
                upstreamReq["model"],
            )
            val upstreamJsonStr = upstreamReq.toString()
            assertTrue("Expected prompt text forwarded to upstream LLM", upstreamJsonStr.contains("Describe this diagram"))
            assertTrue(
                "Expected multimodal image data URL forwarded to upstream LLM, got: $upstreamJsonStr",
                upstreamJsonStr.contains("data:image/png;base64,iVBORw0KGgo="),
            )

            // 3. resumeRemoteConversation (GET /observe) against live nola web
            val resumeReq = remoteResumeRequest(
                hostId = hostId,
                clientId = clientId,
                key = RemoteRunKey(conversationId, "run-live-1", identity.assistantMessageId),
            )
            val resumedEvents = service.resumeRemoteConversation(baseUrl, liveToken, resumeReq).toList()
            assertTrue("Idle conversation GET /observe should return JSON without throwing", resumedEvents.isEmpty())

            val observed = service.observeRemoteConversation(baseUrl, liveToken, resumeReq)
            assertEquals(JsonPrimitive(hostId), observed["source_host_id"])
            assertEquals(JsonPrimitive(conversationId), observed["conversation_id"])
            assertEquals(JsonPrimitive(clientId), observed["client_id"])
            assertNotNull("Expected observe result object", observed["result"])

            // Verify GET /observe did NOT append a duplicate user message
            val messagesCheck = runNolaCli(
                nolaBin = nolaBin,
                homeDir = homeDir,
                workDir = workDir,
                args = listOf("--db", dbFile.absolutePath, "--project", workDir.absolutePath, "sessions", "show", conversationId, "--json"),
            )
            assertTrue("Expected original prompt persisted in session", messagesCheck.contains("Describe this diagram"))

            // 4. expertMessageCommand (operation preserved on encrypted wire & decrypted error response from live nola web)
            // Note: live `POST /api/mobile/agent-rooms` dispatches `dispatch_agent_room_command`, which currently
            // rejects `"parent"`, `"admit"`, and `"manifest"` with encrypted 400 `unsupported agent_room operation '<op>'`.
            // Checking the exact operation name in the decrypted error proves BOTH that `operation` survived the wire
            // AND that `WebHostApiServiceImpl` decrypts AES-256-GCM error envelopes instead of surfacing raw ciphertext.
            for (op in listOf("parent", "admit", "manifest")) {
                val resp = service.expertMessageCommand(
                    hostBaseUrl = baseUrl,
                    token = liveToken,
                    operation = op,
                    request = buildJsonObject {
                        put("expected_host_id", hostId)
                        put("client_id", clientId)
                        put("conversation_id", conversationId)
                    },
                )
                assertEquals(JsonPrimitive(true), resp["ok"])
                assertEquals(JsonPrimitive(hostId), resp["host_id"])
                assertEquals(JsonPrimitive(clientId), resp["client_id"])
                val resObj = resp["result"]?.jsonObject
                assertEquals(JsonPrimitive(op), resObj?.get("operation"))
            }

            // Also verify 4xx encrypted error responses are decrypted instead of surfacing raw ciphertext.
            val errFailure = runCatching {
                service.agentRoomCommand(
                    hostBaseUrl = baseUrl,
                    token = liveToken,
                    request = buildJsonObject {
                        put("type", "nonexistent_room_op_check")
                        put("expected_host_id", hostId)
                    },
                )
            }.exceptionOrNull()
            val errMsg = errFailure?.message.orEmpty()
            assertNotNull("Expected 400 response for nonexistent agent_room op", errFailure)
            assertFalse("Error body must be decrypted, not raw AES-256-GCM ciphertext: $errMsg", errMsg.contains("\"ciphertext\""))
            assertTrue("Expected decrypted unsupported_command message, got: $errMsg", errMsg.contains("nonexistent_room_op_check"))

            // Also verify authenticated encrypted POST /api/mobile/agent-rooms 200 OK round-trips
            val roomDir = service.agentRoomCommand(
                hostBaseUrl = baseUrl,
                token = liveToken,
                request = buildJsonObject {
                    put("type", "directory")
                    put("expected_host_id", hostId)
                },
            )
            assertEquals(JsonPrimitive(true), roomDir["ok"])
            assertEquals(JsonPrimitive(hostId), roomDir["host_id"])
        } finally {
            client.close()
            webProc?.let { p ->
                p.destroy()
                if (!p.waitFor(5, TimeUnit.SECONDS)) p.destroyForcibly()
            }
            upstreamServer.close()
            upstreamExecutor.shutdownNow()
            tempRoot.deleteRecursively()
        }
    }

    @Test
    fun realJniNolaCoreDecodesTodayFactsListActivityAndConfigOnlyModelConnections() {
        val jniLibPath = requireNotNull(System.getProperty("nola.test.modelPrivacyLibrary")) {
            "Expected nola.test.modelPrivacyLibrary system property from Gradle test task"
        }
        assertTrue("Expected compiled JNI library at $jniLibPath", File(jniLibPath).isFile)
        System.load(jniLibPath)

        assertEquals("Expected NolaCore Application ABI version 1", 1, AgentCoreNative.nativeApplicationAbiVersion())
        assertEquals("Expected NolaCore Application protocol version 1", 1, AgentCoreNative.nativeApplicationProtocolVersion())

        val repoRoot = locateRepoRoot()
        val nolaBin = File(repoRoot, "target/debug/nola")
        assertTrue("Expected compiled nola binary at ${nolaBin.absolutePath}", nolaBin.isFile)

        val tempRoot = createPrivateTempDir("nola-android-jni-e2e-")
        val homeDir = File(tempRoot, "home").apply { mkdirs() }
        val workDir = File(tempRoot, "workspace").apply { mkdirs() }
        val dbFile = File(tempRoot, "nola-jni.db")

        // 1. Seed a KV_APPLIED_CONFIG-only connection (no `model_connection` module record) via real `nola config apply`
        val configFile = File(tempRoot, "cfg-only.json")
        configFile.writeText(
            """
            {
              "llm": {
                "default_model": {
                  "connection": "cfg-only-jni",
                  "model": "gpt-5-jni"
                },
                "connections": [
                  {
                    "id": "cfg-only-jni",
                    "endpoint": "https://cfg-only.example.invalid/v1",
                    "protocol": "responses",
                    "models": [
                      { "id": "gpt-5-jni" }
                    ]
                  }
                ]
              }
            }
            """.trimIndent(),
        )
        runNolaCli(
            nolaBin = nolaBin,
            homeDir = homeDir,
            workDir = workDir,
            args = listOf("--db", dbFile.absolutePath, "--project", workDir.absolutePath, "config", "apply", configFile.absolutePath),
        )

        val runtime = openJvmAndroidApplicationRuntime(dbFile)
        try {
            // 2. Seed sessions and automation_run via real JNI mutations
            runtime.v2CreateSession(title = "Waiting Sync", id = "conv-jni-wait")
            runtime.v2CreateSession(title = "Completed Sync", id = "conv-jni-done")
            val nowMs = System.currentTimeMillis()
            runtime.v2UpsertModuleRecord(
                kind = "automation_run",
                record = buildJsonObject {
                    put("id", "auto-jni-1")
                    put("automation_id", "auto-daily")
                    put("title", "Morning Brief")
                    put("summary", "Generated digest")
                    put("status", "completed")
                    put("scheduled_for_ms", nowMs)
                    put("updated_at_ms", nowMs)
                },
            )

            // Seed open_waits:conv-jni-wait directly in SQLite `kv` so conv-jni-wait has needs_response=true
            seedKvRow(dbFile, "open_waits:conv-jni-wait", """[{"wait_id":"wait-jni-1"}]""", nowMs)

            // 3. Real JNI load_today_facts decoded through production TodayPendingResponseReader
            val todayReader = TodayPendingResponseReader(runtime)
            val snapshot = todayReader.read()
            assertTrue("Expected needsAttention=true when open_waits exists", snapshot.needsAttention)
            val waitFact = snapshot.conversations.firstOrNull { it.conversationId == "conv-jni-wait" }
            val doneFact = snapshot.conversations.firstOrNull { it.conversationId == "conv-jni-done" }
            assertNotNull("Expected conv-jni-wait in real load_today_facts output", waitFact)
            assertNotNull("Expected conv-jni-done in real load_today_facts output", doneFact)
            assertEquals("Waiting Sync", waitFact!!.title)
            assertTrue("Expected conv-jni-wait needsResponse=true", waitFact.needsResponse)
            assertEquals(1, waitFact.openWaits.size)
            assertEquals("Completed Sync", doneFact!!.title)
            assertFalse("Expected conv-jni-done needsResponse=false", doneFact.needsResponse)
            assertTrue(doneFact.openWaits.isEmpty())

            // 4. Real JNI list_activity(needs_response = false) MUST return waiting + non-waiting + automation_run
            val allActivity = runtime.executeQuery(
                buildJsonObject {
                    put("type", "list_activity")
                    put("needs_response", false)
                },
            ).jsonObject
            val allEntries = allActivity.getValue("entries").jsonArray.map { it.jsonObject }
            assertEquals("Expected 3 activity entries (wait + done + automation)", 3, allEntries.size)
            assertTrue(
                "Expected conv-jni-wait with needs_response=true",
                allEntries.any { it["id"] == JsonPrimitive("conv-jni-wait") && it["needs_response"]?.jsonPrimitive?.boolean == true },
            )
            assertTrue(
                "Expected conv-jni-done with needs_response=false",
                allEntries.any { it["id"] == JsonPrimitive("conv-jni-done") && it["needs_response"]?.jsonPrimitive?.boolean == false },
            )
            assertTrue(
                "Expected auto-jni-1 with source='automation'",
                allEntries.any { it["id"] == JsonPrimitive("auto-jni-1") && it["source"] == JsonPrimitive("automation") },
            )

            // And needs_response = true strictly filters to waiting conversations
            val waitingOnly = runtime.executeQuery(
                buildJsonObject {
                    put("type", "list_activity")
                    put("needs_response", true)
                },
            ).jsonObject.getValue("entries").jsonArray.map { it.jsonObject }
            assertEquals(1, waitingOnly.size)
            assertEquals(JsonPrimitive("conv-jni-wait"), waitingOnly[0]["id"])

            // 5. Real JNI list_connections (including KV_APPLIED_CONFIG-only connection) through ModelConnectionRepository
            val inventory = ModelConnectionRepository(runtime).load()
            val cfgConn = inventory.connections.firstOrNull { it.id == "cfg-only-jni" }
            assertNotNull(
                "Expected KV_APPLIED_CONFIG-only connection 'cfg-only-jni' decoded by ModelConnectionRepository",
                cfgConn,
            )
            assertEquals("https://cfg-only.example.invalid/v1", cfgConn!!.address)
            assertTrue(cfgConn.enabled)
            assertTrue(
                "Expected model 'gpt-5-jni' in decoded connection catalog, got: ${cfgConn.models}",
                cfgConn.models.contains("gpt-5-jni"),
            )
        } finally {
            runtime.close()
            tempRoot.deleteRecursively()
        }
    }

    private fun locateRepoRoot(): File =
        generateSequence(File(".").absoluteFile) { it.parentFile }
            .firstOrNull { File(it, "Cargo.toml").isFile && File(it, "apps/web").isDirectory }
            ?: error("Could not locate Nola repository root from ${File(".").absolutePath}")

    private fun createPrivateTempDir(prefix: String): File =
        Files.createTempDirectory(
            prefix,
            PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rwx------")),
        ).toFile()

    private fun findFreeLoopbackPort(): Int =
        ServerSocket(0, 1, InetAddress.getByName("127.0.0.1")).use { it.localPort }

    private fun runNolaCli(nolaBin: File, homeDir: File, workDir: File, args: List<String>): String {
        val pb = ProcessBuilder(listOf(nolaBin.absolutePath) + args)
        pb.directory(workDir)
        pb.redirectErrorStream(true)
        pb.environment().apply {
            put("HOME", homeDir.absolutePath)
            put("NO_PROXY", "127.0.0.1,localhost")
            put("no_proxy", "127.0.0.1,localhost")
        }
        val proc = pb.start()
        val output = proc.inputStream.bufferedReader(Charsets.UTF_8).readText()
        assertTrue("nola CLI timed out: $args", proc.waitFor(30, TimeUnit.SECONDS))
        assertEquals("nola CLI failed ($args): $output", 0, proc.exitValue())
        return output
    }

    private suspend fun waitForLiveHealth(
        client: HttpClient,
        baseUrl: String,
        proc: Process,
        logFile: File,
    ): JsonObject {
        val deadline = System.currentTimeMillis() + 15_000L
        var lastError: String? = null
        while (System.currentTimeMillis() < deadline) {
            if (!proc.isAlive) {
                val logs = logFile.takeIf(File::isFile)?.readText().orEmpty()
                error("nola web exited prematurely (code=${proc.exitValue()}): $logs")
            }
            val attempt = runCatching {
                val resp = client.get("$baseUrl/api/health")
                if (resp.status.value == 200) {
                    json.parseToJsonElement(resp.body<String>()).jsonObject
                } else {
                    lastError = "HTTP ${resp.status.value}"
                    null
                }
            }
            attempt.getOrNull()?.let { return it }
            attempt.exceptionOrNull()?.let { lastError = it.message }
            kotlinx.coroutines.delay(150)
        }
        val logs = logFile.takeIf(File::isFile)?.readText().orEmpty()
        error("Timed out waiting for live nola web /api/health (lastError=$lastError). Logs:\n$logs")
    }

    private fun openJvmAndroidApplicationRuntime(dbFile: File): AndroidApplicationRuntime {
        val unsafeClass = Class.forName("sun.misc.Unsafe")
        val unsafeField = unsafeClass.getDeclaredField("theUnsafe").apply { isAccessible = true }
        val unsafe = unsafeField.get(null)
        val allocateInstance = unsafeClass.getMethod("allocateInstance", Class::class.java)
        val runtime = allocateInstance.invoke(unsafe, AndroidApplicationRuntime::class.java) as AndroidApplicationRuntime
        val handle = AgentCoreNative.nativeApplicationOpen(dbFile.absolutePath.encodeToByteArray())
        assertTrue("nativeApplicationOpen returned 0 for ${dbFile.absolutePath}", handle != 0L)

        fun setField(name: String, value: Any?) {
            val f = AndroidApplicationRuntime::class.java.getDeclaredField(name)
            f.isAccessible = true
            f.set(runtime, value)
        }
        setField("databaseFile", dbFile)
        setField("json", Json { ignoreUnknownKeys = true })
        setField("lock", Any())
        setField("handle", handle)
        setField("submissionJournal", ApplicationSubmissionJournal(File(dbFile.path + ".pending-commands.json")))
        setField("recoveredReceipts", mutableListOf<JsonObject>())
        return runtime
    }

    private fun seedKvRow(dbFile: File, key: String, valueJson: String, updatedAtMs: Long) {
        val pb = ProcessBuilder(
            "python3",
            "-c",
            "import sqlite3, sys; c=sqlite3.connect(sys.argv[1]); c.execute('INSERT OR REPLACE INTO kv(key, value_json, updated_at_ms) VALUES (?, ?, ?)', (sys.argv[2], sys.argv[3], int(sys.argv[4]))); c.commit(); c.close()",
            dbFile.absolutePath,
            key,
            valueJson,
            updatedAtMs.toString(),
        )
        pb.redirectErrorStream(true)
        val proc = pb.start()
        val out = proc.inputStream.bufferedReader(Charsets.UTF_8).readText()
        assertTrue("python3 sqlite3 seed timed out", proc.waitFor(10, TimeUnit.SECONDS))
        assertEquals("python3 sqlite3 seed failed: $out", 0, proc.exitValue())
    }

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
}
