package com.nola.data.remote.api

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.sse.SseEvent
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.AndroidAgentPermissionMode
import java.util.TimeZone
import kotlinx.serialization.json.JsonObject

interface WebHostApiService {
    suspend fun expertMessageCommand(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject { error("Host-owned expert messages are unavailable") }

    suspend fun auxiliaryCommand(hostBaseUrl: String, token: String, action: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Auxiliary transport is unavailable")
    }
    suspend fun agentRoomCommand(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Host-owned agent room transport is unavailable")
    }
    suspend fun goalControl(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject { error("Remote Goal transport is unavailable") }
    suspend fun conversationProgress(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Remote progress transport is unavailable")
    }
    suspend fun submissionFrontier(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Submission frontier transport is unavailable")
    }
    suspend fun permissionConfiguration(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Permission configuration transport is unavailable")
    }
    suspend fun observeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Remote conversation observation is not implemented")
    }
    fun resumeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): kotlinx.coroutines.flow.Flow<SseEvent> {
        error("Exact remote Run resume is not implemented")
    }
    suspend fun generativeUi(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Generative UI transport is unavailable")
    }
    suspend fun readAssistantJournal(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject {
        error("Assistant journal transport is not implemented")
    }
    suspend fun requestWebManagementUrl(
        hostBaseUrl: String,
        token: String,
        target: String = "/settings#channels",
        pinnedCertificateSha256: String = ""
    ): String {
        error("Web Agent Host browser handoff is not implemented")
    }

    suspend fun getAgentSettings(
        hostBaseUrl: String,
        token: String,
        pinnedCertificateSha256: String = ""
    ): AgentSettingsResponse

    suspend fun submitAgentPermissionDecision(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String = ""
    )

    suspend fun cancelAdmittedRemoteSubmission(
        hostBaseUrl: String,
        token: String,
        identity: RemoteSubmissionIdentity,
        requestDigest: String,
        pinnedCertificateSha256: String = ""
    ) {
        error("Web Agent Host cancellation is not implemented")
    }

    suspend fun submitAgentDeviceToolResult(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String = ""
    )

    suspend fun taskCancellationCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String = ""): JsonObject {
        error("Web Agent Host task cancellation is not implemented")
    }

    suspend fun taskGuidanceCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String = ""): JsonObject {
        error("Web Agent Host task guidance is not implemented")
    }

    fun streamAgentTaskEvents(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        afterCursor: Long,
        pinnedCertificateSha256: String = "",
    ): kotlinx.coroutines.flow.Flow<SseEvent> = kotlinx.coroutines.flow.emptyFlow()

    suspend fun remoteSubmission(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject

    suspend fun prepareToolPlan(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String = ""): JsonObject

    fun streamBoundAgentRequest(
        hostBaseUrl: String,
        token: String,
        submission: FrozenRemoteSubmission,
        pinnedCertificateSha256: String = "",
    ): kotlinx.coroutines.flow.Flow<SseEvent>
}
