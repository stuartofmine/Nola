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
    private val hostRequests = ArrayDeque<HostAgentPermissionRequest>()
    private var deviceToolRequest: SseEvent.DeviceToolRequest? = null
    private var instructionSubmissionId: String? = null
    private var rememberLocalGrant = false

    fun hostRequest(): HostAgentPermissionRequest? = hostRequests.firstOrNull()

    fun hostRequests(): List<HostAgentPermissionRequest> = hostRequests.toList()

    fun findHostRequest(requestId: String): HostAgentPermissionRequest? =
        hostRequests.firstOrNull { it.id == requestId }

    fun setHostRequest(request: HostAgentPermissionRequest?) {
        hostRequests.clear()
        if (request != null) hostRequests.addLast(request)
    }

    fun setHostRequests(requests: List<HostAgentPermissionRequest>) {
        hostRequests.clear()
        requests.forEach { enqueueHostRequest(it) }
    }

    fun enqueueHostRequest(request: HostAgentPermissionRequest) {
        val index = hostRequests.indexOfFirst { it.id == request.id }
        if (index >= 0) {
            hostRequests[index] = request
        } else {
            hostRequests.addLast(request)
        }
    }

    fun prependHostRequest(request: HostAgentPermissionRequest) {
        if (hostRequests.none { it.id == request.id }) {
            hostRequests.addFirst(request)
        }
    }

    fun takeHostRequest(requestId: String? = null): HostAgentPermissionRequest? {
        if (requestId == null) return hostRequests.removeFirstOrNull()
        return removeHostRequest(requestId)
    }

    fun removeHostRequest(requestId: String): HostAgentPermissionRequest? {
        val index = hostRequests.indexOfFirst { it.id == requestId }
        return if (index >= 0) hostRequests.removeAt(index) else null
    }

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
        hostRequests.clear()
        deviceToolRequest = null
        instructionSubmissionId = null
        rememberLocalGrant = false
    }
}
