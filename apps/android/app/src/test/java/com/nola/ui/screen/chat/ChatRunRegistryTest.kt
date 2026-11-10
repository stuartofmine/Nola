package com.nola.ui.screen.chat

import com.nola.core.AndroidAgentPermissionRequest
import com.nola.data.local.db.dao.AgentTraceDao
import com.nola.data.local.db.entity.AgentTraceEventEntity
import com.nola.data.repository.AgentTraceRepository
import com.nola.domain.model.ProcessingState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonNull

@OptIn(ExperimentalCoroutinesApi::class)
class ChatRunRegistryTest {
    @Test fun preparedDraftStorageBindingCannotChangeCanonicalExecutionPublicationOwner() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val drafts = ConversationComposerDrafts()
        val input = drafts.forConversation(null)
        input.text.value = "retained input"
        val initialOwner = drafts.ownerKey(null)
        var selectedId: String? = null
        var visible = ChatUiState()
        val registry = ChatRunRegistry(backgroundScope, traces()) { owner, state ->
            if (owner.ownerKey == drafts.ownerKey(selectedId)) visible = state
        }
        try {
            val session = registry.beginExecution(initialOwner, visible)
            drafts.bindConversation("prepared", initialOwner)
            registry.bind(session, "prepared")
            selectedId = "prepared"
            session.state.value = session.state.value.copy(conversationId = "prepared", currentResponse = "actual live text")
            runCurrent()
            assertEquals("conversation:prepared", session.ownerKey)
            assertEquals(session.ownerKey, drafts.ownerKey(selectedId))
            assertEquals("actual live text", visible.currentResponse)
            assertSame(input, drafts.forConversation(selectedId))
            assertEquals("retained input", drafts.forConversation(selectedId).text.value)
        } finally { registry.clear(); drafts.close(); runCurrent(); Dispatchers.resetMain() }
    }

    @Test fun completedGenerationWakesQueuedInputOnlyAfterLocalLeaseAndJobRelease() = runTest {
        var completions = 0
        lateinit var registry: ChatRunRegistry
        registry = ChatRunRegistry(backgroundScope, traces(), generationCompleted = { owner ->
            assertFalse(owner.runtime.hasOutstandingGeneration())
            assertTrue(registry.localAvailable(owner.ownerKey))
            completions++
        }) { _, _ -> }
        val owner = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        registry.bind(owner, "a")
        val finish = CompletableDeferred<Unit>()
        assertTrue(registry.acquireLocal(owner))
        registry.start(owner) { finish.await() }
        runCurrent()
        assertFalse(owner.state.value.executionTeardownPending)
        // Simulates an action arriving after Core sealed the Run, while local
        // finalization is still in flight. It must be retried after teardown.
        assertTrue(owner.runtime.hasOutstandingGeneration())
        assertEquals(0, completions)
        owner.hasCommittedTerminalSeal = true
        finish.complete(Unit)
        runCurrent()
        assertEquals(1, completions)
        registry.clear()
    }

    @Test fun sealedCancellationKeepsComposerBlockedUntilPhysicalJobFinishes() = runTest {
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, _ -> }
        val owner = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val other = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val teardown = CompletableDeferred<Unit>()
        registry.start(owner) {
            owner.markTerminalCommitted()
            owner.successorCancellationRequested = true
            owner.state.value = owner.state.value.copy(processingState = ProcessingState.Idle)
            teardown.await()
        }
        runCurrent()
        assertTrue(owner.hasCommittedTerminalSeal)
        assertTrue(owner.state.value.executionTeardownPending)
        assertTrue(owner.retainsLoadedPresentation())
        assertFalse(other.state.value.executionTeardownPending)
        teardown.complete(Unit)
        runCurrent()
        assertFalse(owner.runtime.hasOutstandingGeneration())
        assertFalse(owner.state.value.executionTeardownPending)
        registry.clear()
    }

    @Test fun backgroundHydrationRemainsOwnedWhileAnotherMemberIsSelected() = runTest {
        var visible = ChatUiState(conversationId = "b")
        val registry = ChatRunRegistry(backgroundScope, traces()) { owner, state ->
            if (owner.ownerKey == "conversation:b") visible = state
        }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        a.loadingPresentation = true
        assertTrue(a.retainsLoadedPresentation())
        val loaded = CompletableDeferred<Unit>()
        val hydration = registry.scope(a).launch {
            loaded.await()
            assertSame(a, registry.current)
            a.state.value = a.state.value.copy(currentResponse = "restored A")
            a.loadingPresentation = false
        }
        runCurrent()
        loaded.complete(Unit)
        runCurrent()
        assertTrue(hydration.isCompleted)
        assertEquals("b", visible.conversationId)
        assertEquals("restored A", a.state.value.currentResponse)
        assertFalse(a.retainsLoadedPresentation())
        registry.clear()
    }
    @Test fun permissionSaveContextKeepsOriginalOwnerAcrossSelectionAndSuspension() = runTest {
        var selected = "conversation:a"
        var visible = ChatUiState()
        val registry = ChatRunRegistry(backgroundScope, traces()) { owner, state -> if (owner.ownerKey == selected) visible = state }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a", permissionSaving = true,
            permissionMode = com.nola.core.AndroidAgentPermissionMode.AllowAll))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val response = CompletableDeferred<Unit>()
        val save = backgroundScope.launch(registry.scope(a).coroutineContext) {
            assertSame(a, registry.current)
            response.await()
            assertSame(a, registry.current)
            a.state.value = a.state.value.copy(permissionPolicyRunId = "run-a", permissionPolicyRevision = 12)
        }
        runCurrent(); selected = b.ownerKey
        b.state.value = b.state.value.copy(currentResponse = "selected B")
        runCurrent()
        // The actual loadConversation fast-restore predicate must preserve this
        // idle edit on A→B→A, not reload old DB defaults over its in-flight save.
        assertTrue(a.retainsLoadedPresentation())
        selected = a.ownerKey
        visible = a.state.value
        assertTrue(visible.permissionSaving)
        assertEquals(com.nola.core.AndroidAgentPermissionMode.AllowAll, visible.permissionMode)
        selected = b.ownerKey
        visible = b.state.value
        response.complete(Unit); runCurrent()
        assertTrue(save.isCompleted)
        assertEquals(12L, a.state.value.permissionPolicyRevision)
        assertNull(b.state.value.permissionPolicyRunId)
        assertEquals("b", visible.conversationId)
        registry.clear()
    }
    private fun traces() = AgentTraceRepository(object : AgentTraceDao {
        override suspend fun append(event: AgentTraceEventEntity) = 1L
        override suspend fun events(traceId: String) = emptyList<AgentTraceEventEntity>()
        override suspend fun latestTraceId(conversationId: String): String? = null
        override suspend fun deleteConversationTrace(conversationId: String) = Unit
    })

    @Test fun cancelledLocalOwnerKeepsLeaseUntilPhysicalTeardownCompletes() = runTest {
        var queueWakes = 0
        val registry = ChatRunRegistry(backgroundScope, traces(), generationCompleted = { queueWakes++ }) { _, _ -> }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val cleanup = CompletableDeferred<Unit>()
        assertTrue(registry.acquireLocal(a))
        registry.start(a) {
            try { awaitCancellation() }
            finally { withContext(NonCancellable) { cleanup.await() } }
        }
        runCurrent(); registry.remove(a.ownerKey); runCurrent()
        assertFalse(registry.localAvailable(b.ownerKey)); assertFalse(registry.acquireLocal(b))
        cleanup.complete(Unit); runCurrent()
        assertEquals(0, queueWakes)
        assertTrue(registry.acquireLocal(b))
        registry.releaseLocal(b); registry.clear()
    }

    @Test fun preparationFailureWithoutCommittedSealReleasesLeaseWithoutRetryLoop() = runTest {
        var queueWakes = 0
        val registry = ChatRunRegistry(backgroundScope, traces(), generationCompleted = { queueWakes++ }) { _, _ -> }
        val owner = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        assertTrue(registry.acquireLocal(owner))
        registry.start(owner) { owner.state.value = owner.state.value.copy(error = "model unavailable") }
        runCurrent()
        assertEquals(0, queueWakes)
        assertFalse(owner.runtime.hasOutstandingGeneration())
        assertTrue(registry.localAvailable(owner.ownerKey))
        registry.clear()
    }

    @Test fun explicitStopSuppressesQueuedSuccessorEvenWhenCancellationSealsNormally() = runTest {
        var queueWakes = 0
        val registry = ChatRunRegistry(backgroundScope, traces(), generationCompleted = { queueWakes++ }) { _, _ -> }
        val owner = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val terminal = CompletableDeferred<Unit>()
        assertTrue(registry.acquireLocal(owner))
        registry.start(owner) { terminal.await() }
        runCurrent()
        owner.successorCancellationRequested = true
        owner.hasCommittedTerminalSeal = true
        terminal.complete(Unit)
        runCurrent()
        assertEquals(0, queueWakes)
        assertFalse(owner.runtime.hasOutstandingGeneration())
        assertTrue(registry.localAvailable(owner.ownerKey))
        registry.clear()
    }

    @Test fun executionContextSurvivesDispatcherHopAndNestedIntentRestoresOriginalOwner() = runTest {
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, _ -> }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        withContext(registry.scope(a).coroutineContext) {
            assertSame(a, registry.current)
            withContext(Dispatchers.Default) { assertSame(a, registry.current) }
            registry.enter(b) { assertSame(b, registry.current) }
            assertSame(a, registry.current)
        }
        assertNull(registry.current)
        registry.clear()
    }

    @Test fun twoApiOwnersContinueIndependentlyAndLateEventsNeverReplaceSelectedOwner() = runTest {
        var selected = "conversation:a"
        var visible = ChatUiState()
        val registry = ChatRunRegistry(backgroundScope, traces()) { owner, value ->
            if (owner.ownerKey == selected) visible = value
        }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        registry.bind(a, "a"); registry.bind(b, "b")
        val finishA = CompletableDeferred<Unit>()
        val finishB = CompletableDeferred<Unit>()
        val aJob = a.runtime.startGeneration(registry.scope(a)) {
            finishA.await()
            assertSame(a, registry.current)
            a.state.value = a.state.value.copy(currentResponse = "late A", processingState = ProcessingState.Streaming)
        }
        val bJob = b.runtime.startGeneration(registry.scope(b)) {
            finishB.await()
            assertSame(b, registry.current)
            b.state.value = b.state.value.copy(currentResponse = "B")
        }
        runCurrent(); assertTrue(aJob.isActive); assertTrue(bJob.isActive)
        selected = b.ownerKey
        b.state.value = b.state.value.copy(currentResponse = "selected B")
        runCurrent()
        registry.enter(b) { assertSame(b, registry.current); b.runtime.cancelGeneration() }
        assertNull(registry.current)
        runCurrent(); assertTrue(aJob.isActive); assertTrue(bJob.isCancelled)
        finishA.complete(Unit); runCurrent()
        assertEquals("late A", a.state.value.currentResponse)
        assertEquals("b", visible.conversationId); assertEquals("selected B", visible.currentResponse)
        selected = a.ownerKey
        assertEquals("late A", registry.find(selected)!!.state.value.currentResponse)
        registry.clear()
    }

    @Test fun localLeaseDoesNotRestrictApiAndOldOwnerCannotReleaseNewLease() = runTest {
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, _ -> }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val gate = CompletableDeferred<Unit>()
        registry.start(a) { gate.await() }
        runCurrent(); assertTrue(registry.acquireLocal(a)); assertFalse(registry.acquireLocal(b))
        // API B does not acquire the device lease and can run while Local A owns it.
        registry.start(b) { gate.await() }
        runCurrent(); assertTrue(a.runtime.hasActiveGeneration()); assertTrue(b.runtime.hasActiveGeneration())
        registry.releaseLocal(a); assertTrue(registry.acquireLocal(b))
        registry.releaseLocal(a); assertFalse(registry.acquireLocal(a))
        registry.clear()
    }

    @Test fun delayedPermissionCallbackResolvesOriginalOwnerAndDeletionRetiresOnlyThatOwner() = runTest {
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, _ -> }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val permission = AndroidAgentPermissionRequest(true, "request-a", "call-a", "read_document", "file", "ask", "read", null)
        a.state.value = a.state.value.copy(agentPermissionRequest = permission)
        assertSame(a, registry.permissionOwner(permission.requestId))
        registry.enter(b) { assertSame(b, registry.current) }
        registry.enter(checkNotNull(registry.permissionOwner(permission.requestId))) {
            assertSame(a, registry.current)
            a.permission.rememberLocalGrant(true)
        }
        assertFalse(b.permission.shouldRememberLocalGrant())
        registry.remove(a.ownerKey)
        assertNull(registry.permissionOwner(permission.requestId)); assertTrue(a.retired)
        assertSame(b, registry.find(b.ownerKey)); assertFalse(b.retired)
        registry.clear()
    }

    @Test fun remoteObservationKeepsOriginalContextWhenOtherOwnerIsRemoved() = runTest {
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, _ -> }
        val a = registry.session("conversation:a", ChatUiState(conversationId = "a"))
        val b = registry.session("conversation:b", ChatUiState(conversationId = "b"))
        val aReply = CompletableDeferred<JsonObject>()
        val bReply = CompletableDeferred<JsonObject>()
        var aQueries = 0; var bQueries = 0; var aAbsent = 0; var bAbsent = 0
        for ((owner, reply) in listOf(a to aReply, b to bReply)) {
            owner.observer.start(registry.scope(owner), owner.ownerKey, "host", "client",
                checkNotNull(owner.state.value.conversationId), null, { !owner.retired },
                query = {
                    assertSame(owner, registry.current)
                    if (owner === a) aQueries++ else bQueries++
                    reply.await().also { assertSame(owner, registry.current) }
                }, catchUp = { false }, onObservation = { _, _ -> },
                onAbsent = { if (owner === a) aAbsent++ else bAbsent++ },
                onUnavailable = { throw it }, resume = { fail("Absent Run cannot resume") })
        }
        runCurrent(); assertEquals(1, aQueries); assertEquals(1, bQueries)
        registry.remove(b.ownerKey)
        aReply.complete(JsonObject(mapOf("source_host_id" to JsonPrimitive("host"), "conversation_id" to JsonPrimitive("a"),
            "client_id" to JsonPrimitive("client"), "result" to JsonNull)))
        runCurrent(); assertEquals(1, aAbsent); assertEquals(0, bAbsent)
        registry.clear()
    }

    @Test fun successorExecutionDoesNotReuseOldProjectionPermissionOrScope() = runTest {
        var visible = ChatUiState()
        val registry = ChatRunRegistry(backgroundScope, traces()) { _, state -> visible = state }
        val original = registry.beginExecution("conversation:a", ChatUiState(conversationId = "a", currentResponse = "original"))
        original.permission.rememberLocalGrant(true)
        runCurrent()
        val successor = registry.beginExecution("conversation:a", ChatUiState(conversationId = "a", currentResponse = "successor"))
        runCurrent()
        assertTrue(original.retired); assertTrue(original.lifetime.isCancelled)
        assertFalse(successor.permission.shouldRememberLocalGrant())
        original.state.value = original.state.value.copy(currentResponse = "late original")
        runCurrent()
        assertEquals("successor", visible.currentResponse)
        assertSame(successor, registry.find(successor.ownerKey))
        registry.clear()
    }
}
