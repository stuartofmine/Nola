package com.nola.ui.screen.chat

import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.HostAgentPermissionRequest

/**
 * Owns transient permission state for host and device-tool requests.
 *
 * Input: validated permission requests and user intent.
 * Output: the request currently eligible for resolution and duplicate-submission guards.
 */
internal class ChatPermissionSession {
    private var hostRequest: HostAgentPermissionRequest? = null
    private var deviceToolRequest: SseEvent.DeviceToolRequest? = null
    private var instructionSubmissionId: String? = null
    private var rememberLocalGrant = false

    fun hostRequest(): HostAgentPermissionRequest? = hostRequest

    fun setHostRequest(request: HostAgentPermissionRequest?) {
        hostRequest = request
    }

    fun takeHostRequest(): HostAgentPermissionRequest? = hostRequest.also { hostRequest = null }

    fun deviceToolRequest(): SseEvent.DeviceToolRequest? = deviceToolRequest

    fun setDeviceToolRequest(request: SseEvent.DeviceToolRequest?) {
        deviceToolRequest = request
    }

    fun takeDeviceToolRequest(): SseEvent.DeviceToolRequest? =
        deviceToolRequest.also { deviceToolRequest = null }

    fun beginInstruction(requestId: String): Boolean {
        if (instructionSubmissionId == requestId) return false
        instructionSubmissionId = requestId
        return true
    }

    fun finishInstruction(requestId: String) {
        if (instructionSubmissionId == requestId) instructionSubmissionId = null
    }

    fun isSubmittingInstruction(requestId: String): Boolean = instructionSubmissionId == requestId

    fun rememberLocalGrant(remember: Boolean) {
        rememberLocalGrant = remember
    }

    fun shouldRememberLocalGrant(): Boolean = rememberLocalGrant

    fun clearLocalGrantPreference() {
        rememberLocalGrant = false
    }

    fun clear() {
        hostRequest = null
        deviceToolRequest = null
        instructionSubmissionId = null
        rememberLocalGrant = false
    }
}
