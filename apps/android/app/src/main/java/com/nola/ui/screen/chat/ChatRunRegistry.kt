package com.nola.ui.screen.chat

import com.nola.data.repository.AgentTraceRepository
import com.nola.data.remote.api.RemoteRunKey
import com.nola.data.remote.api.RemoteSubmissionIntent
import com.nola.data.remote.api.RemoteSubmissionReceipt
import com.nola.data.local.AndroidJournalTurn
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.llm.provider.WebAgentProvider
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asContextElement
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

/** Run effects and projection belong to this owner, never to the selected tab. */
internal class ChatRunSession(
    var ownerKey: String,
    initial: ChatUiState,
    traceRepository: AgentTraceRepository,
    parentJob: Job?,
) {
    val lifetime = SupervisorJob(parentJob)
    val state = MutableStateFlow(initial)
    val preparationBase = initial
    var dispatchStarted = false
    var hasCommittedTerminalSeal = false
    var successorCancellationRequested = false
    fun markTerminalCommitted() {
        hasCommittedTerminalSeal = true
        state.update { it.copy(executionTeardownPending = runtime.hasOutstandingGeneration()) }
    }
    var submittedDraft: ConversationComposerDrafts.SubmissionTicket? = null
    var submittedDraftRunId: String? = null
    var draftRetired = false
    val runtime = ChatTurnRuntime()
    val permission = ChatPermissionSession()
    val taskObservation = RemoteTaskObservation()
    val observer = RemoteConversationObserver()
    val trace = AgentTraceCoordinator(traceRepository)
    val capabilities = CapabilityActivationBuffer()
    var configuration: WebAgentProvider.Config? = null
    var initialKey: RemoteRunKey? = null
    var visibleHeads: List<JsonObject> = emptyList()
    var submission: RemoteSubmissionIntent? = null
    var admitted = false
    var permissionSourceUnconfirmed = false
    var permissionOwnerProjectionRunId: String? = null
    var permissionObservedIdentity: com.nola.data.remote.api.RemoteSubmissionIdentity? = null
    var permissionObservedRecovery: com.nola.data.remote.api.RemoteRunRecovery? = null
    val permissionHydration = com.nola.data.local.PermissionOwnerHydration()
    var receipt: RemoteSubmissionReceipt? = null
    var journalTurn: AndroidJournalTurn? = null
    var journalTransport: (suspend (JsonObject) -> JsonObject)? = null
    var journalProjection: ConversationProjectionV1? = null
    @Volatile var retired = false
    var presentationJob: kotlinx.coroutines.Job? = null
    var loadingPresentation = false
    fun retainsLoadedPresentation(): Boolean = loadingPresentation || runtime.hasOutstandingGeneration() ||
        state.value.processingState.blocksInput || state.value.permissionSaving
    fun dispose() {
        retired = true
        runtime.cancelGeneration()
        observer.reset()
        taskObservation.reset()
        permission.clear()
        presentationJob?.cancel()
        lifetime.cancel()
    }
}

/** The physical local model outlives any particular screen/ViewModel. */
private object ChatDeviceExecutionLease {
    private var owner: ChatRunSession? = null
    @Synchronized fun available(): Boolean = owner == null
    @Synchronized fun acquire(session: ChatRunSession): Boolean {
        if (owner != null && owner !== session) return false
        owner = session
        return true
    }
    @Synchronized fun release(session: ChatRunSession) { if (owner === session) owner = null }
}

/** Main-dispatcher registry; a coroutine carries its original session across suspension. */
internal class ChatRunRegistry(
    private val scope: CoroutineScope,
    private val traceRepository: AgentTraceRepository,
    private val generationCompleted: (ChatRunSession) -> Unit = {},
    private val sealedRunObserved: (ChatRunSession) -> Unit = {},
    private val publish: (ChatRunSession, ChatUiState) -> Unit,
) {
    private val sessions = mutableMapOf<String, ChatRunSession>()
    private val execution = ThreadLocal<ChatRunSession?>()
    val current: ChatRunSession? get() = execution.get()
    fun find(key: String): ChatRunSession? = sessions[key]?.takeUnless { it.retired }
    fun permissionOwner(requestId: String): ChatRunSession? = sessions.values.filter {
        !it.retired && (it.state.value.agentPermissionRequest?.requestId == requestId ||
            it.permission.deviceToolRequest()?.id == requestId || it.permission.hostRequest()?.id == requestId)
    }.singleOrNull()
    fun session(key: String, initial: ChatUiState): ChatRunSession = find(key) ?: create(key, initial)
    fun localAvailable(key: String): Boolean = ChatDeviceExecutionLease.available()
    fun beginExecution(key: String, initial: ChatUiState): ChatRunSession {
        find(key)?.let {
            check(!it.runtime.hasOutstandingGeneration()) { "This owner already has an active execution" }
            it.dispose()
        }
        return create(key, initial)
    }
    private fun create(key: String, initial: ChatUiState): ChatRunSession {
        val session = ChatRunSession(key, initial, traceRepository, scope.coroutineContext[Job])
        sessions[key] = session
        session.presentationJob = scope.launch { session.state.collect { if (!session.retired) publish(session, it) } }
        return session
    }
    fun bind(session: ChatRunSession, conversationId: String) {
        check(!session.retired)
        val key = "conversation:$conversationId"
        val existing = find(key)
        check(existing == null || existing === session) { "Conversation already has an execution owner" }
        sessions.remove(session.ownerKey, session)
        session.ownerKey = key
        sessions[key] = session
        session.runtime.bindOwner(conversationId)
    }
    fun acquireLocal(session: ChatRunSession): Boolean {
        return ChatDeviceExecutionLease.acquire(session)
    }
    fun releaseLocal(session: ChatRunSession) { ChatDeviceExecutionLease.release(session) }
    fun start(session: ChatRunSession, block: suspend CoroutineScope.() -> Unit): Job {
        val job = session.runtime.startGeneration(scope(session), block)
        job.invokeOnCompletion { failure ->
            releaseLocal(session)
            session.state.update { it.copy(executionTeardownPending = false) }
            // A UI action may commit after the last queue promotion but before
            // this job releases its execution lease. Wake only after teardown.
            if (failure == null && session.hasCommittedTerminalSeal && !session.retired && !session.successorCancellationRequested) {
                generationCompleted(session)
            }
            if (session.hasCommittedTerminalSeal && !session.retired) sealedRunObserved(session)
        }
        return job
    }
    fun scope(session: ChatRunSession): CoroutineScope = CoroutineScope(scope.coroutineContext + session.lifetime + execution.asContextElement(session))
    fun <T> enter(session: ChatRunSession, block: () -> T): T {
        val previous = execution.get()
        execution.set(session)
        return try { block() } finally { execution.set(previous) }
    }
    fun remove(key: String) {
        sessions.remove(key)?.dispose()
    }
    fun clear() { sessions.values.toSet().forEach(ChatRunSession::dispose); sessions.clear() }
}
