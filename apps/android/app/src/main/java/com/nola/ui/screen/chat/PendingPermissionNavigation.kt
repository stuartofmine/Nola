package com.nola.ui.screen.chat

/** Presentation only. Never derives permission from text, tool name or execution location. */
internal data class PendingPermissionNavigationTarget(
    val count: Int,
    val toolName: String,
    val itemIndex: Int,
    val itemKey: String,
)

internal fun ChatUiState.pendingPermissionNavigation(messageCount: Int): PendingPermissionNavigationTarget? {
    val host = permissionRequest
    // Once the system picker/permission UI has taken over, its local card is
    // deliberately absent; do not offer a shortcut to a nonexistent item.
    val local = agentPermissionRequest.takeIf { agentPermissionAction == null }
    val count = (if (host != null) 1 else 0) + (if (local != null) 1 else 0)
    if (count == 0) return null
    val emptyStateCount = if (messageCount == 0 && !processingState.isGenerating) 1 else 0
    val sourceStatusCount = if (compactionSourceStatus != CompactionSourceStatus.Idle) 1 else 0
    return PendingPermissionNavigationTarget(
        count = count,
        toolName = host?.toolName ?: requireNotNull(local).toolName,
        // Permission cards follow the empty/source-status rows, messages and tail anchor.
        itemIndex = emptyStateCount + sourceStatusCount + messageCount + 1,
        itemKey = host?.let { "host_permission_${it.id}" }
            ?: "local_agent_permission_${requireNotNull(local).requestId}",
    )
}
