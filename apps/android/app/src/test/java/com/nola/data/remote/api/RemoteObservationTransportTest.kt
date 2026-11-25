package com.nola.data.remote.api

import com.nola.agent.trace.AgentTraceReducer
import com.nola.agent.trace.AgentTraceSession
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.decodeSsePayload
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class RemoteObservationTransportTest {

    @Test
    fun v2SsePayloadDecodesAllAgentEventsDirectly() {
        val started = decodeSsePayload(
            """{"type":"session_started","session_id":"conv-1","model":"gpt-test"}""",
            null,
        )
        assertIs<SseEvent.V2SessionStarted>(started)
        assertEquals("conv-1", started.sessionId)

        val reasoning = decodeSsePayload("""{"type":"reasoning_delta","delta":"step 1"}""", null)
        assertIs<SseEvent.V2ReasoningDelta>(reasoning)
        assertEquals("step 1", reasoning.delta)

        val text = decodeSsePayload("""{"type":"text_delta","delta":"hello v2"}""", null)
        assertIs<SseEvent.V2TextDelta>(text)
        assertEquals("hello v2", text.delta)

        val toolStart = decodeSsePayload("""{"type":"tool_started","call_id":"tc-1","name":"read_file"}""", null)
        assertIs<SseEvent.V2ToolStarted>(toolStart)
        assertEquals("read_file", toolStart.name)

        val toolEnd = decodeSsePayload(
            """{"type":"tool_finished","call_id":"tc-1","name":"read_file","is_error":false,"summary":"ok"}""",
            null,
        )
        assertIs<SseEvent.V2ToolFinished>(toolEnd)
        assertEquals(false, toolEnd.isError)
        assertEquals("ok", toolEnd.summary)

        val uiSurface = decodeSsePayload(
            """{"type":"ui_surface","snapshot":{"revision":1,"phase":"ready","summary":"Deploy Form"}}""",
            null,
        )
        assertIs<SseEvent.V2UiSurface>(uiSurface)
        assertEquals("ready", (uiSurface.snapshot["phase"] as? kotlinx.serialization.json.JsonPrimitive)?.content)

        val perm = decodeSsePayload(
            """{"type":"permission_requested","request_id":"req-1","tool":"write_file","reason":"Requires confirmation","arguments":{"path":"a.txt"}}""",
            null,
        )
        assertIs<SseEvent.V2PermissionRequested>(perm)
        assertEquals("req-1", perm.requestId)
        assertEquals("write_file", perm.tool)

        val question = decodeSsePayload(
            """{"type":"user_question","request_id":"q-1","question":"Target env?","options":["Staging","Production"]}""",
            null,
        )
        assertIs<SseEvent.V2UserQuestion>(question)
        assertEquals("q-1", question.requestId)
        assertEquals(listOf("Staging", "Production"), question.options)

        val completed = decodeSsePayload(
            """{"type":"turn_completed","session_id":"conv-1","final_text":"hello v2"}""",
            null,
        )
        assertIs<SseEvent.V2TurnCompleted>(completed)
        assertEquals("hello v2", completed.finalText)

        assertNull(decodeSsePayload("[DONE]", null))
    }

    @Test
    fun v2WebHostStreamCallsDirectConversationMessagesEndpoint() = runTest {
        val seenPaths = mutableListOf<String>()
        val seenAuth = mutableListOf<String?>()
        val mockEngine = MockEngine { req ->
            seenPaths += req.url.encodedPath
            seenAuth += req.headers["Authorization"]
            respond(
                content = "data: {\"type\":\"text_delta\",\"delta\":\"Hi from v2\"}\n\n" +
                    "data: {\"type\":\"turn_completed\",\"session_id\":\"c-42\",\"final_text\":\"Hi from v2\"}\n\n" +
                    "data: [DONE]\n\n",
                status = HttpStatusCode.OK,
                headers = headersOf("Content-Type", "text/event-stream"),
            )
        }
        val client = HttpClient(mockEngine)
        val sse = SseClient(client)
        val url = webHostV2ApiUrl("http://127.0.0.1:5001", "chat/conversations/c-42/messages")
        val events = sse.stream(
            url = url,
            bodyJson = """{"content":"hello"}""",
            headers = mapOf("Authorization" to "Bearer tok", "X-Nola-Client" to "android"),
            mobileCryptoToken = null,
            requestClient = client,
        ).toList()

        assertEquals(listOf("/api/chat/conversations/c-42/messages"), seenPaths)
        assertEquals(listOf<String?>("Bearer tok"), seenAuth)
        assertEquals(2, events.size)
        assertIs<SseEvent.V2TextDelta>(events[0])
        assertEquals("Hi from v2", (events[0] as SseEvent.V2TextDelta).delta)
        assertIs<SseEvent.V2TurnCompleted>(events[1])
    }

    @Test
    fun v2SseTraceAdapterMapsInteractiveAndCompactionEvents() {
        val session = AgentTraceSession(
            traceId = "trace-v2",
            runId = "run-v2",
            conversationId = "conv-v2",
            messageId = "msg-v2",
            turnId = "turn-v2",
        )

        val permEvent = session.fromSse(
            SseEvent.V2PermissionRequested(
                requestId = "perm-1",
                tool = "write_file",
                reason = "Needs approval",
                arguments = buildJsonObject { put("path", "notes.md") },
            )
        )
        assertNotNull(permEvent)
        assertEquals("tool.permission.requested", permEvent.kind)
        assertEquals("waiting", permEvent.status)
        assertEquals("perm-1", permEvent.payload.permissionRequestId)
        assertEquals("write_file", permEvent.payload.toolName)
        assertEquals("Needs approval", permEvent.payload.summary)

        val questionEvent = session.fromSse(
            SseEvent.V2UserQuestion(
                requestId = "q-1",
                question = "Deploy target?",
                options = listOf("Staging", "Production"),
            )
        )
        assertNotNull(questionEvent)
        assertEquals("phase.changed", questionEvent.kind)
        assertEquals("waiting", questionEvent.status)
        assertEquals("awaiting_user_input", questionEvent.payload.phase)
        assertEquals("Deploy target?", questionEvent.payload.summary)
        assertEquals("Staging, Production", questionEvent.payload.detail)

        val surfaceSnapshot = buildJsonObject {
            put("identity", buildJsonObject { put("surface_id", "surf-1") })
            put("phase", "ready")
            put("summary", "Deploy Form")
        }
        val surfaceEvent = session.fromSse(SseEvent.V2UiSurface(snapshot = surfaceSnapshot))
        assertNotNull(surfaceEvent)
        assertEquals("tool.progress", surfaceEvent.kind)
        assertEquals("surf-1", surfaceEvent.spanId)
        assertEquals("render_ui_surface", surfaceEvent.payload.toolName)
        assertEquals("Deploy Form", surfaceEvent.payload.summary)
        assertEquals(surfaceSnapshot, surfaceEvent.payload.result)

        val compactedEvent = session.fromSse(SseEvent.V2Compacted(summary = "Compacted 12 messages"))
        assertNotNull(compactedEvent)
        assertEquals("context.compacted", compactedEvent.kind)
        assertEquals("compacting", compactedEvent.payload.phase)
        assertEquals("Compacted 12 messages", compactedEvent.payload.summary)

        val projection = AgentTraceReducer.replay(listOf(permEvent, questionEvent, surfaceEvent, compactedEvent))
        assertEquals("processing", projection.phase)
        assertTrue(projection.tools.any { it.spanId == "perm-1" && it.status == "permission_required" })
        assertTrue(projection.tools.any { it.spanId == "surf-1" && it.toolName == "render_ui_surface" })
    }

    @Test
    fun prepareToolPlanAndGenerativeUiRouteToExpectedV2Endpoints() = runTest {
        val requests = CopyOnWriteArrayList<Pair<String, String>>()
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val acceptThread = thread(isDaemon = true) {
            repeat(3) {
                val socket = runCatching { server.accept() }.getOrNull() ?: return@thread
                socket.use { client ->
                    val reader = BufferedReader(InputStreamReader(client.getInputStream()))
                    val requestLine = reader.readLine().orEmpty()
                    val parts = requestLine.split(" ")
                    if (parts.size >= 2) requests += parts[0] to parts[1]
                    var contentLength = 0
                    while (true) {
                        val header = reader.readLine() ?: break
                        if (header.isEmpty()) break
                        if (header.startsWith("Content-Length:", ignoreCase = true)) {
                            contentLength = header.substringAfter(":").trim().toIntOrNull() ?: 0
                        }
                    }
                    if (contentLength > 0) {
                        val bodyChars = CharArray(contentLength)
                        var read = 0
                        while (read < contentLength) {
                            val n = reader.read(bodyChars, read, contentLength - read)
                            if (n <= 0) break
                            read += n
                        }
                    }
                    val body = """{"ok":true}"""
                    val response = "HTTP/1.1 200 OK\r\n" +
                        "Content-Type: application/json\r\n" +
                        "Content-Length: ${body.toByteArray().size}\r\n" +
                        "Connection: close\r\n\r\n" +
                        body
                    client.getOutputStream().write(response.toByteArray())
                    client.getOutputStream().flush()
                }
            }
        }
        try {
            val baseUrl = "http://127.0.0.1:${server.localPort}"
            val mockClient = HttpClient(MockEngine { respond("""{}""", HttpStatusCode.OK) })
            val api = WebHostApiServiceImpl(
                httpClient = mockClient,
                sseClient = SseClient(mockClient),
                httpClientFactory = WebHostHttpClientFactory(mockClient),
            )
            api.prepareToolPlan(
                hostBaseUrl = baseUrl,
                token = "tok",
                request = buildJsonObject { put("conversation_id", "c-1") },
                pinnedCertificateSha256 = "",
            )
            api.generativeUi(
                hostBaseUrl = baseUrl,
                token = "tok",
                operation = "queued-input",
                request = buildJsonObject { put("conversation_id", "c-1") },
                pinnedCertificateSha256 = "",
            )
            api.generativeUi(
                hostBaseUrl = baseUrl,
                token = "tok",
                operation = "queued-input/cancel",
                request = buildJsonObject { put("conversation_id", "c-1") },
                pinnedCertificateSha256 = "",
            )

            assertEquals(
                listOf(
                    "POST" to "/api/agent/tool-plan/prepare",
                    "GET" to "/api/chat/conversations/c-1/events",
                    "POST" to "/api/chat/conversations/c-1/cancel",
                ),
                requests.toList(),
            )
        } finally {
            server.close()
            acceptThread.join(2_000)
        }
    }
}
