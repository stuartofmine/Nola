package com.nola.data.remote.api

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
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull

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
}
