// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Host permission resume identity and typed decision payloads.

package com.nola.ui.screen.chat

import com.nola.domain.model.HostAgentPermissionRequest

internal fun shouldAbortHostPermissionResume(
    conversationId: String?,
    conversationExists: Boolean
): Boolean = hostPermissionResumeConversationId(conversationId) == null || !conversationExists

internal fun hostPermissionResumeConversationId(conversationId: String?): String? =
    conversationId?.takeIf { it.isNotBlank() }

internal fun hostPermissionDecisionPayload(
    request: HostAgentPermissionRequest,
    allow: Boolean,
    rememberForConversation: Boolean = false,
): Map<String, Any> = buildMap {
    put("id", request.id)
    put("request_id", request.id)
    put("tool_name", request.toolName)
    put("match", request.match)
    put("resolution", mapOf(
        "type" to "authorization",
        "decision" to when {
            !allow -> "deny"
            rememberForConversation -> "allow_conversation"
            else -> "allow_once"
        },
    ))
    request.nonce?.let { put("nonce", it) }
    request.hostId?.let { put("host_id", it) }
    request.clientId?.let { put("client_id", it) }
    request.sessionId?.let { put("session_id", it) }
    request.protocolVersion?.let { put("protocol_version", it) }
}

internal fun hostPermissionInstructionPayload(request: HostAgentPermissionRequest, instruction: String): Map<String, Any> =
    hostPermissionDecisionPayload(request, false).toMutableMap().apply {
        put("resolution", mapOf("type" to "intervention", "intervention" to mapOf(
            "type" to "provide_instruction", "instruction" to com.nola.core.normalizedPermissionInstruction(request.id, request.permissionInstructions, instruction))))
    }

internal fun permissionResponseStillCurrent(expectedId: String, currentId: String?): Boolean = expectedId == currentId
