package com.nola.ui.screen.chat

import com.nola.core.AndroidAgentPermissionAction
import com.nola.core.AndroidAgentPermissionRequest
import com.nola.domain.model.HostAgentPermissionRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PendingPermissionNavigationTest {
    private val local = AndroidAgentPermissionRequest(
        allowConversation = true,
        requestId = "local-request", callId = "call", toolName = "read_document",
        capability = "file", authorization = "ask", rationale = "Read selected document", scope = null,
    )

    @Test
    fun noRequestDoesNotInventPermissionFromToolText() {
        assertNull(ChatUiState(currentResponse = "Please authorize read_document").pendingPermissionNavigation(1))
    }

    @Test
    fun hostAndLocalShareCountAndFirstVisibleCardPosition() {
        val state = ChatUiState(
            permissionRequest = HostAgentPermissionRequest(true, "host-request", "shell"),
            agentPermissionRequest = local,
            compactionSourceStatus = CompactionSourceStatus.Loading,
        )
        assertEquals(PendingPermissionNavigationTarget(2, "shell", 42, "host_permission_host-request"), state.pendingPermissionNavigation(40))
        assertEquals(PendingPermissionNavigationTarget(1, "read_document", 42, "local_agent_permission_local-request"),
            state.copy(permissionRequest = null).pendingPermissionNavigation(40))
    }

    @Test
    fun emptyTranscriptAccountsForEmptyStateAndTail() {
        assertEquals(PendingPermissionNavigationTarget(1, "read_document", 2, "local_agent_permission_local-request"),
            ChatUiState(agentPermissionRequest = local).pendingPermissionNavigation(0))
    }

    @Test
    fun systemPermissionHandoffHasNoDanglingNavigationTarget() {
        val state = ChatUiState(
            agentPermissionRequest = local,
            agentPermissionAction = AndroidAgentPermissionAction.OpenDocument(local.requestId, false),
        )
        assertNull(state.pendingPermissionNavigation(3))
        assertEquals(PendingPermissionNavigationTarget(1, "shell", 4, "host_permission_host"),
            state.copy(permissionRequest = HostAgentPermissionRequest(true, "host", "shell"))
                .pendingPermissionNavigation(3))
    }
}
