package com.nola.agent.trace

import com.nola.data.remote.sse.SseEvent
import java.util.UUID
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentTraceContractEndToEndTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun submissionReceiptNeverCreatesTraceActivityOrPermissionChallenge() {
        val wire = """{"type":"submission_receipt","receipt":{
            "protocol_version":1,"identity":{"source_host_id":"host","client_id":"client",
            "conversation_id":"conversation","submission_id":"submission","user_message_id":"user","assistant_message_id":"assistant"},
            "request_digest":"${"a".repeat(64)}","phase":"prepared","admission":null,"rejection_code":null},
            "tool_name":"write_file","nonce":"not-a-permission","message":"Allow write?"}"""
        val decoded = requireNotNull(com.nola.data.remote.sse.decodeSsePayload(wire, null))
        assertTrue(decoded is SseEvent.SubmissionReceipt)
        assertFalse(decoded is SseEvent.PermissionRequest)
        val session = AgentTraceSession("trace", "run", "conversation", "assistant", "turn")
        val before = session.projection
        repeat(2) {
            assertEquals(null, session.fromSse(decoded))
            assertEquals(before, session.projection)
        }
        assertTrue(session.projection.tools.isEmpty())
        assertEquals("Receipt must not consume an audit sequence", 1L, session.start().sequence)
    }

    @Test
    fun compactionOwnsStageAndModelEventsDoNotCreateTools() {
        var state = AgentTraceProjection()
        fun apply(sequence: Long, kind: String, payload: AgentTracePayload) {
            state = AgentTraceReducer.reduce(state, AgentTraceEvent(
                traceId = "trace", runId = "run", conversationId = "conversation", turnId = "turn",
                eventId = "event-$sequence", sequence = sequence, emittedAt = "2026-09-21T00:00:00Z",
                kind = kind, spanId = "model-effect", payload = payload,
            )).projection
        }
        apply(1, "phase.changed", AgentTracePayload(phase = "compacting"))
        apply(2, "tool.progress", AgentTracePayload(summary = "model_attempt_started"))
        apply(3, "tool.progress", AgentTracePayload(summary = "model_response_provenance"))
        assertEquals("compacting", state.phase)
        assertTrue(state.tools.isEmpty())
        apply(4, "context.compacted", AgentTracePayload(phase = "compacted"))
        assertEquals("processing", state.phase)
        apply(5, "tool.progress", AgentTracePayload(summary = "model_retry"))
        assertEquals("model_retry", state.phase)
        apply(6, "tool.progress", AgentTracePayload(summary = "model_attempt_started"))
        assertEquals("processing", state.phase)
        assertTrue(state.tools.isEmpty())
        apply(7, "phase.changed", AgentTracePayload(phase = "compacting"))
        apply(8, "run.cancelled", AgentTracePayload(reason = "cancelled"))
        assertEquals(null, state.phase)
    }

    @Test
    fun sharedLongTaskFixtureProjectsOneStableAssistantTimeline() {
        val events = sharedFixture()
        val projection = AgentTraceReducer.replay(events)

        assertEquals(9L, projection.lastSequence)
        assertTrue(events.none { it.kind in setOf("assistant.delta", "assistant.reset", "thinking.delta") })
        assertEquals(2, projection.tools.size)
        assertEquals("web_search", projection.tools[0].toolName)
        assertEquals("2026年7月伊朗局势", projection.tools[0].arguments["query"]?.let { (it as JsonPrimitive).content })
        assertEquals("search", projection.tools[0].resources.single().operation)
        assertEquals("Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md", projection.tools[1].resources.single().uri)
        assertEquals(8192, projection.usage?.usedTokens)
        assertEquals(131072, projection.usage?.contextWindowTokens)
        assertEquals("succeeded", projection.status)
        assertEquals(1, projection.ignoredDuplicateCount)
        assertTrue(projection.missingSequences.isEmpty())
    }

    @Test
    fun permissionResumeKeepsControlAndToolIdentity() {
        val session = AgentTraceSession(
            traceId = "trace-permission",
            runId = "run-permission",
            conversationId = "conversation",
            messageId = "assistant",
            turnId = "turn",
        )
        val events = listOfNotNull(
            session.start(),
            session.fromSse(
                SseEvent.PermissionRequest(
                    allowConversation = true,
                    id = "call-write",
                    toolName = "write_file",
                    args = mapOf("path" to "report.md"),
                    match = "write",
                    reason = "需要写入报告",
                    message = "允许写入？",
                )
            ),
            session.permissionResolved("call-write", "write_file", allowed = true),
            session.completed(),
        )
        val projection = AgentTraceReducer.replay(events)

        assertEquals(1, projection.tools.size)
        assertEquals("trace-permission:call-write", projection.tools.single().stableId)
        assertEquals("running", projection.tools.single().status)
        assertTrue(projection.isTerminal)
    }

    @Test
    fun duplicateAndGapAreExplicitAndDoNotRollBackControl() {
        val original = sharedFixture().first().copy(kind = "phase.changed")
        val first = original.copy(sequence = 1, eventId = "first", payload = AgentTracePayload(phase = "processing"))
        val gap = original.copy(sequence = 3, eventId = "third", payload = AgentTracePayload(phase = "compacting"))
        val late = original.copy(sequence = 2, eventId = "second", payload = AgentTracePayload(phase = "processing"))

        var projection = AgentTraceProjection()
        projection = AgentTraceReducer.reduce(projection, first).projection
        projection = AgentTraceReducer.reduce(projection, first).projection
        projection = AgentTraceReducer.reduce(projection, gap).projection
        projection = AgentTraceReducer.reduce(projection, late).projection

        assertEquals("compacting", projection.phase)
        assertEquals(setOf(2L), projection.missingSequences)
        assertEquals(1, projection.ignoredDuplicateCount)
        assertEquals(1, projection.ignoredOutOfOrderCount)
    }

    @Test
    fun stableMarkdownCommitsBlocksWithoutCompletionTreeSwap() {
        val first = streamingMarkdownProjection("## 标题\n\n第一段仍在", complete = false)
        assertEquals(1, first.committedBlocks.size)
        assertEquals("第一段仍在", first.mutableTail?.markdown)

        val second = streamingMarkdownProjection(
            "## 标题\n\n第一段已经完成。\n\n```kotlin\nval answer = 42\n```\n",
            complete = false,
        )
        assertEquals("block-0", first.committedBlocks.first().id)
        assertEquals("block-0", second.committedBlocks.first().id)
        assertTrue(second.committedBlocks.any { it.markdown.contains("第一段已经完成") })
        // A closed fence without a following blank line remains the same live
        // paragraph owner until completion, matching the Apple partitioner.
        assertTrue(second.mutableTail?.markdown?.contains("val answer = 42") == true)

        val completed = streamingMarkdownProjection(second.visibleText, complete = true)
        assertEquals(second.visibleText, completed.visibleText)
        assertFalse(completed.committedBlocks.isEmpty())
        assertEquals(second.mutableTail?.id, completed.committedBlocks.last().id)
    }

    @Test
    fun canonicalCoreFieldNamesDecodeWithoutPlatformAliases() {
        val event = json.decodeFromString<AgentTraceEvent>(
            """
            {
              "schema_version": 1,
              "trace_id": "trace-contract",
              "run_id": "run-contract",
              "conversation_id": "conversation-contract",
              "turn_id": "turn-contract",
              "event_id": "event-contract",
              "sequence": 1,
              "emitted_at": "2026-07-19T00:00:00Z",
              "kind": "tool.permission.requested",
              "status": "running",
              "visibility": "summary",
              "payload": {
                "permission_request_id": "permission-contract",
                "usage": {
                  "used_tokens": 16,
                  "context_window_tokens": 128000
                },
                "validation": {
                  "status": "pending",
                  "level": "warning",
                  "summary": "needs review",
                  "next_action": "review"
                }
              }
            }
            """.trimIndent(),
        )

        assertEquals("permission-contract", event.payload.permissionRequestId)
        assertEquals(128000, event.payload.usage?.contextWindowTokens)
        assertEquals("pending", event.payload.validation?.status)
        assertEquals("warning", event.payload.validation?.level)
        assertEquals("review", event.payload.validation?.nextAction)
    }

    @Test
    fun acceptedSemanticNoOpAdvancesJournalCursorWithoutPublishingUi() {
        val original = sharedFixture().first { it.kind == "run.started" }
        val first = original.copy(sequence = 1, eventId = "run-started-1")
        val repeated = original.copy(sequence = 2, eventId = "run-started-2")

        val initial = AgentTraceReducer.reduce(AgentTraceProjection(), first)
        val semanticNoOp = AgentTraceReducer.reduce(initial.projection, repeated)

        assertTrue(initial.applied)
        assertFalse(semanticNoOp.applied)
        assertEquals(2, semanticNoOp.projection.lastSequence)
        assertTrue("run-started-2" in semanticNoOp.projection.seenEventIds)
        assertEquals(initial.projection.status, semanticNoOp.projection.status)
    }

    private fun sharedFixture(): List<AgentTraceEvent> {
        val resource = checkNotNull(javaClass.classLoader?.getResource("long_task_trace_v1.json")) {
            "Core Agent Trace fixture is not on the Android JVM test classpath"
        }
        return json.decodeFromString(resource.readText())
    }
}
