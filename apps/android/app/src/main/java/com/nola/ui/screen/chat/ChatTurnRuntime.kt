package com.nola.ui.screen.chat

import com.nola.core.AgentCoreSnapshot
import com.nola.core.AgentCoreTaskProjectionReducer
import com.nola.core.AgentCoreTaskUpdate
import com.nola.core.DeviceAgentCoordinator
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Owns process-local state for one active assistant turn.
 *
 * Input: generation jobs, a device coordinator, and Core projection events.
 * Output: cancellable runtime handles and reduced UI state; no persistence is performed here.
 */
internal class ChatTurnRuntime {
    var conversationOwner: String? = null
        private set
    fun bindOwner(conversationId: String) {
        conversationOwner = conversationId
    }
    fun owns(conversationId: String?): Boolean = conversationId != null && conversationOwner == conversationId
    private var generationJob: Job? = null
    private var deviceCoordinator: DeviceAgentCoordinator? = null

    fun hasActiveGeneration(): Boolean = generationJob?.isActive == true
    fun hasOutstandingGeneration(): Boolean = generationJob?.isCompleted == false

    fun replaceGeneration(job: Job) {
        generationJob?.cancel()
        generationJob = job
    }

    fun startGeneration(scope: CoroutineScope, block: suspend CoroutineScope.() -> Unit): Job {
        val job = scope.launch(start = CoroutineStart.LAZY, block = block)
        replaceGeneration(job)
        job.invokeOnCompletion { clearGenerationIfCurrent(job) }
        job.start()
        return job
    }

    fun cancelGeneration() {
        generationJob?.cancel()
    }

    fun clearGenerationIfCurrent(job: Job?) {
        if (job != null && job.isCompleted && generationJob == job) generationJob = null
    }

    fun bindDeviceCoordinator(coordinator: DeviceAgentCoordinator) {
        deviceCoordinator = coordinator
    }

    fun clearDeviceCoordinator(coordinator: DeviceAgentCoordinator) {
        if (deviceCoordinator === coordinator) deviceCoordinator = null
    }

    fun deviceCoordinator(): DeviceAgentCoordinator? = deviceCoordinator

    fun reduceTask(state: ChatUiState, event: AgentCoreTaskUpdate): ChatUiState? {
        val coreSessionId = state.agentCoreSnapshot?.sessionId ?: state.coreTaskProjection.sessionId
        if (coreSessionId == null || coreSessionId != event.sessionId) return null
        val reduction = AgentCoreTaskProjectionReducer.reduce(state.coreTaskProjection, event)
        return reduction.projection.takeIf { reduction.applied }
            ?.let { state.copy(coreTaskProjection = it) }
    }

    fun applySnapshot(state: ChatUiState, snapshot: AgentCoreSnapshot): ChatUiState {
        val pendingPermissionIds = snapshot.permissions
            ?.pending
            ?.mapTo(mutableSetOf()) { it.requestId }
        val visiblePermission = if (pendingPermissionIds == null) {
            state.agentPermissionRequest
        } else {
            state.agentPermissionRequest?.takeIf { it.requestId in pendingPermissionIds }
        }
        return state.copy(
            agentCoreSnapshot = snapshot,
            coreTaskProjection = if (snapshot.sessionId != null &&
                state.coreTaskProjection.sessionId != snapshot.sessionId) {
                com.nola.core.AgentCoreTaskProjection(sessionId = snapshot.sessionId)
            } else state.coreTaskProjection,
            coreSkillProjection = if (snapshot.sessionId != null &&
                state.coreSkillProjection.sessionId != snapshot.sessionId) {
                com.nola.core.AgentCoreSkillRuntimeProjection(sessionId = snapshot.sessionId)
            } else state.coreSkillProjection,
            agentPermissionRequest = visiblePermission,
            agentPermissionAction = if (visiblePermission == null) null else state.agentPermissionAction,
        )
    }

}
