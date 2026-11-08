package com.nola.ui.screen.chat

import com.nola.data.remote.api.*
import com.nola.data.remote.sse.SseStatusException
import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.decodeSsePayload
import com.nola.domain.model.ProcessingState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*
import kotlin.test.*

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class RemoteConversationObserverTest {
    private val key = RemoteRunKey("conversation", "durable-run", "original-root")
    private val identity = buildJsonObject {
        put("source_store_epoch", "epoch"); put("conversation_id", key.conversationId)
        put("run_id", key.runId); put("assistant_message_id", key.assistantMessageId)
    }
    private val admission = buildJsonObject { put("identity", identity); put("sequence", "1"); put("record_digest", "a".repeat(64)) }
    private fun response(recovery: String = "observe", worker: String = "running", terminal: Boolean = false): JsonObject = buildJsonObject {
        put("source_host_id", "host"); put("conversation_id", key.conversationId); put("client_id", "client")
        put("result", buildJsonObject {
            put("protocol_version", 1); put("client_id", "client"); put("conversation_id", key.conversationId); put("run_id", key.runId)
            put("assistant_message_id", key.assistantMessageId); put("status", if (terminal) "sealed" else "running")
            put("worker", buildJsonObject { put("state", worker); put("generation", "original-generation") })
            put("recovery", recovery); put("admission", admission)
            put("terminal_seal", if (terminal) buildJsonObject {
                put("identity", identity); put("admission", admission); put("seal_digest", "b".repeat(64))
                put("protocol_version", 2); put("core_session_id", "core"); put("transition_id", "terminal")
                put("checkpoint_revision", "2"); put("core_revision", "2"); put("event_cursor", "9")
                put("completed_at_ms", 2); put("entry_count", "1"); put("heads_count", "1"); put("child_receipt_count", "0")
                put("manifest_digest", "c".repeat(64)); put("result_digest", "d".repeat(64))
                put("result", buildJsonObject {
                    put("result_id", "decision"); put("conversation_id", key.conversationId); put("run_id", key.runId)
                    put("originating_submission_id", "submission")
                    put("input", buildJsonObject { put("watermark", "1"); put("applications_digest", "e".repeat(64)) })
                    put("decided_at_revision", "2"); put("decision_event_cursor", "8")
                    put("requirement_revision", "1"); put("evidence_frontier", "7")
                    put("value", buildJsonObject { put("kind", "answered"); put("answer_part_refs", JsonArray(listOf(JsonPrimitive("answer")))) })
                })
            } else JsonNull)
            put("permissions", JsonArray(emptyList())); put("can_submit", terminal)
            put("submission_receipt", JsonNull)
            put("context_usage", JsonNull)
        })
    }

    @Test fun unresolvedSubmissionHasNoRunAuthorityUntilExactReceiptReconciliation() = runTest {
        val observer = RemoteConversationObserver()
        var selected = true
        var admitted = false
        var reconciliations = 0
        val requests = mutableListOf<JsonObject>()
        observer.start(this, "selected", "host", "client", key.conversationId, null, { selected },
            query = {
                assertTrue(admitted); requests += it
                assertEquals("observe", it.getValue("operation").jsonPrimitive.content)
                assertEquals(key.runId, it.getValue("expected_run_id").jsonPrimitive.content)
                assertEquals(key.assistantMessageId, it.getValue("assistant_message_id").jsonPrimitive.content)
                response()
            }, catchUp = { selected = false; false }, onObservation = { _, _ -> },
            onAbsent = { fail("Unresolved submission is not absent") }, onUnavailable = { throw it },
            resume = { fail("Reconciliation must not execute input") },
            pause = { if (reconciliations == 2) admitted = true },
            reconcileSubmission = { reconciliations++; admitted }, admittedKey = { key.takeIf { admitted } })
        runCurrent()
        assertEquals(3, reconciliations); assertEquals(1, requests.size)
        observer.reset()
    }

    @Test fun streamingLoadIdentityRemainsEligibleForObservation() {
        val loads = ConversationLoadCoordinator()
        val generation = loads.beginRequest()
        assertTrue(loads.shouldCommit(generation))
        assertTrue(loads.isCurrent(generation))
        loads.beginRequest()
        assertFalse(loads.isCurrent(generation))
    }

    @Test fun coldDiscoveryPinsExactRunAndAbsentWorkerDoesNotAuthorizeResume() = runTest {
        val observer = RemoteConversationObserver()
        val requests = mutableListOf<JsonObject>()
        var selected = true
        var received = 0
        observer.start(this, "selected", "host", "client", key.conversationId, null, { selected },
            query = { requests += it; response(worker = "absent") },
            catchUp = { assertEquals(key, it.key); received++; false },
            onObservation = { observation, committed -> assertFalse(observation.canSubmit || committed) },
            onAbsent = { fail("Worker absence is not durable Run absence") },
            onUnavailable = { throw it }, resume = { fail("observe must not start a worker") },
            pause = { if (received == 2) selected = false })
        runCurrent()
        assertEquals(2, received)
        assertEquals("discover", requests[0].getValue("operation").jsonPrimitive.content)
        assertEquals("observe", requests[1].getValue("operation").jsonPrimitive.content)
        assertEquals(key.runId, requests[1].getValue("expected_run_id").jsonPrimitive.content)
        assertEquals(key.assistantMessageId, requests[1].getValue("assistant_message_id").jsonPrimitive.content)
        observer.reset()
    }

    @Test fun resumeConflictReturnsToExactObserveWithoutAnotherExecutionRequest() = runTest {
        val observer = RemoteConversationObserver()
        var reads = 0
        var resumes = 0
        var terminalCommitted = false
        observer.start(this, "selected", "host", "client", key.conversationId, key, { true },
            query = { request ->
                assertEquals(key.runId, request.getValue("expected_run_id").jsonPrimitive.content)
                reads++
                if (reads < 3) response("resume_required", "absent") else response("terminal", "absent", true)
            }, catchUp = { it.recovery == RemoteRunRecovery.Terminal },
            onObservation = { value, committed -> if (value.canSubmit) { assertTrue(committed); terminalCommitted = true } },
            onAbsent = { fail("Exact Run cannot become idle by absence") }, onUnavailable = { throw it },
            resume = { assertEquals(key, it); resumes++; throw SseStatusException(409) }, pause = {})
        runCurrent()
        assertEquals(3, reads); assertEquals(1, resumes); assertTrue(terminalCommitted)
        observer.reset()
    }

    @Test fun httpFailureKeepsUnsettledStateAndUsesBoundedBackoff() = runTest {
        val observer = RemoteConversationObserver()
        val delays = mutableListOf<Long>()
        var selected = true
        var errors = 0
        observer.start(this, "selected", "host", "client", key.conversationId, null, { selected },
            query = { throw WebHostStatusException(404) }, catchUp = { fail("No evidence") },
            onObservation = { _, _ -> fail("HTTP error is not a snapshot") },
            onAbsent = { fail("4xx is not idle") }, onUnavailable = { errors++ },
            resume = { fail("No resume authority") }, pause = { delays += it; if (delays.size == 8) selected = false })
        runCurrent()
        assertEquals(8, errors)
        assertEquals(listOf(1_000L, 2_000L, 4_000L, 8_000L, 16_000L, 30_000L, 30_000L, 30_000L), delays)
        observer.reset()
    }

    @Test fun foregroundRestartRetainsPinnedRunAndUncertainResumeAuthority() = runTest {
        val observer = RemoteConversationObserver()
        val paused = CompletableDeferred<Unit>()
        var resumes = 0
        var selected = true
        val queries = mutableListOf<String>()
        fun start(second: Boolean) = observer.start(this, "selected", "host", "client", key.conversationId, null, { selected },
            query = { queries += it.getValue("operation").jsonPrimitive.content; response("resume_required", "absent") },
            catchUp = { false }, onObservation = { _, _ -> }, onAbsent = { fail("Not absent") },
            onUnavailable = { }, resume = { resumes++; throw java.io.IOException("lost resume acknowledgement") },
            pause = { if (second) selected = false else paused.await() })
        start(false); runCurrent(); observer.stop(); runCurrent()
        start(true); runCurrent()
        assertEquals(listOf("discover", "observe"), queries)
        assertEquals(1, resumes)
        observer.reset()
    }

    @Test fun newLoadGenerationCancelsOldObserverWithoutChangingItsExecutionAuthority() = runTest {
        val observer = RemoteConversationObserver()
        val paused = CompletableDeferred<Unit>()
        var oldLoad = true
        var newLoad = true
        var resumes = 0
        val operations = mutableListOf<String>()
        val query: suspend (JsonObject) -> JsonObject = {
            operations += it.getValue("operation").jsonPrimitive.content
            response("resume_required", "absent")
        }
        observer.start(this, "same-conversation", "host", "client", key.conversationId, null, { oldLoad },
            query, { false }, { _, _ -> }, { fail("Not absent") }, {}, { resumes++; throw SseStatusException(409) },
            { paused.await() })
        runCurrent(); oldLoad = false
        // The first job is still paused/active, but its load witness is stale.
        observer.start(this, "same-conversation", "host", "client", key.conversationId, null, { newLoad },
            query, { false }, { _, _ -> }, { fail("Not absent") }, {}, { resumes++ }, { newLoad = false })
        runCurrent()
        assertEquals(listOf("discover", "observe"), operations)
        assertEquals(1, resumes)
        observer.reset()
    }

    @Test fun observationRejectsSourceRunAndRootReplacementAndExactNull() {
        assertFails { decodeRemoteRunObservation(response(), "foreign", "client", key.conversationId) }
        assertFails { decodeRemoteRunObservation(response(), "host", "other-client", key.conversationId) }
        assertFails { decodeRemoteRunObservation(response(), "host", "client", key.conversationId, key.copy(runId = "other")) }
        assertFails { decodeRemoteRunObservation(response(), "host", "client", key.conversationId, key.copy(assistantMessageId = "latest")) }
        val absent = buildJsonObject { put("source_host_id", "host"); put("conversation_id", "conversation"); put("client_id", "client"); put("result", JsonNull) }
        assertNull(decodeRemoteRunObservation(absent, "host", "client", key.conversationId))
        assertFails { decodeRemoteRunObservation(absent, "host", "foreign-client", key.conversationId) }
        assertFails { decodeRemoteRunObservation(absent, "host", "client", "foreign-conversation") }
        assertFails { decodeRemoteRunObservation(absent, "host", "client", key.conversationId, key) }
        assertFalse(remoteResumeRequest("host", "client", key).containsKey("submission_id"))
    }

    @Test fun originalPermissionNonceEntersExistingUiAndResolutionContractAfterColdRead() {
        val permission = Json.parseToJsonElement("""{"type":"permission_request", "allow_conversation": true,"id":"original-permission","canonical_run_id":"durable-run","tool_name":"write_file","protocol_version": 4,"nonce":"original-nonce","host_id":"host","client_id":"client","session_id":"conversation","expires_at":200,"permission_instructions":[]}""").jsonObject
        val base = response()
        val value = base.getValue("result").jsonObject
        val observed = decodeRemoteRunObservation(JsonObject(base + ("result" to JsonObject(value +
            ("permissions" to JsonArray(listOf(permission)))))), "host", "client", key.conversationId, key)!!
        val event = decodeSsePayload(observed.permissions.single().toString(), null) as SseEvent.PermissionRequest
        val state = acceptHostPermissionEvent(ChatUiState(conversationId = key.conversationId,
            processingState = ProcessingState.Streaming), event, key.conversationId, "host", "client", 100)!!
        val request = state.permissionRequest!!
        assertEquals("original-nonce", request.nonce)
        assertEquals("original-permission", hostPermissionDecisionPayload(request, true, false)["request_id"])
        assertEquals("original-nonce", hostPermissionDecisionPayload(request, true, false)["nonce"])
        assertTrue(state.processingState.blocksInput)
    }
}
