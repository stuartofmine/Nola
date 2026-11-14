package com.nola.data.remote.dto

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

const val HOST_PERMISSION_PROTOCOL_VERSION = 1

@Serializable
data class AuthResponse(
    val accessToken: String,
    val expiresIn: Long? = null,
    val userId: String,
    val email: String? = null,
    val displayName: String? = null,
    val authProvider: String? = null
)

@Serializable
data class UserInfoResponse(
    val userId: String,
    val email: String? = null,
    val displayName: String? = null,
    val authProvider: String? = null
)

@Serializable
data class ModelsListResponse(
    val `data`: List<ModelData>? = null,
    val models: List<String>? = null
)

@Serializable
data class ModelData(
    val id: String,
    val `object`: String? = null,
    val ownedBy: String? = null
)

@Serializable
data class SourceSyncRequest(
    val timezone: String,
    val sources: List<SourceItem>,
    val deletedSourceIds: List<String> = emptyList()
)

@Serializable
data class SourceItem(
    val sourceId: String,
    val name: String,
    val url: String,
    val format: String,
    val enabled: Boolean = true,
    val scheduleFrequency: String = "none",
    val scheduledTimeIso: String? = null,
    val scheduledWeekday: Int? = null,
    val timezone: String = "UTC",
    val userPrompt: String? = null,
    val workflowPrompt: String? = null,
    val llmModel: String? = null,
    val skills: List<String>? = null,
    val updatedAt: String? = null
)

@Serializable
data class SourceSyncResponse(
    val accepted: List<String>? = null,
    val serverTime: String? = null
)

@Serializable
data class DeviceRegisterRequest(
    val deviceToken: String,
    val timezone: String = "UTC",
    val pushEnabled: Boolean = true,
    val platform: String = "android"
)

@Serializable
data class DeviceRegisterResponse(
    val deviceId: String? = null
)

@Serializable
data class NotificationsListResponse(
    val notifications: List<NotificationDto>,
    val count: Int = 0,
    val unreadCount: Int = 0
)

@Serializable
data class NotificationDto(
    val id: String,
    val title: String,
    val body: String? = null,
    val source: String? = null,
    val type: String? = null,
    val dedupeKey: String? = null,
    val payload: Map<String, String>? = null,
    val isRead: Boolean = false,
    val createdAt: String? = null,
    val readAt: String? = null
)

@Serializable
data class UnreadCountResponse(
    val unreadCount: Int = 0
)

@Serializable
data class AgentSettingsResponse(
    @SerialName("source_store_epoch") val sourceStoreEpoch: String? = null,
    @SerialName("enable_local_dev_tools") val enableLocalDevTools: Boolean = false,
    @SerialName("allowed_dirs") val allowedDirs: List<String>? = null,
    @SerialName("bash_prefixes") val bashPrefixes: List<String>? = null,
    @SerialName("bash_allowed_prefixes") val bashAllowedPrefixes: List<String>? = null,
    @SerialName("agent_llm_endpoint") val agentLlmEndpoint: String? = null,
    @SerialName("has_agent_llm_api_key") val hasAgentLlmApiKey: Boolean = false,
    @SerialName("agent_llm_model") val agentLlmModel: String? = null,
    @SerialName("long_term_memory") val longTermMemory: List<String> = emptyList(),
    @SerialName("user_preferences") val userPreferences: List<String> = emptyList(),
    @SerialName("mcp_servers") val mcpServers: List<McpServerDto>? = null,
    @SerialName("uploaded_skills") val uploadedSkills: List<UploadedContentDto>? = null,
    @SerialName("uploaded_documents") val uploadedDocuments: List<UploadedContentDto>? = null,
    @SerialName("computer_use") val computerUse: ComputerUseStatusDto? = null,
    @SerialName("native_browser") val nativeBrowser: NativeBrowserStatusDto? = null,
    @SerialName("host_identity") val hostIdentity: HostIdentityDto? = null,
    @SerialName("execution_backends") val executionBackends: List<ExecutionBackendDto> = emptyList(),
    @SerialName("execution_backend") val executionBackend: String? = null,
    @SerialName("model_profiles") val modelProfiles: List<HostModelProfileDto> = emptyList(),
    @SerialName("default_chat_model_revision") val defaultChatModelRevision: Long? = null,
    @SerialName("default_chat_model_resolution") val defaultChatModelResolution: HostModelResolutionDto? = null,
    val messaging: MessagingStatusDto? = null
)

@Serializable
data class MessagingStatusDto(
    val worker: MessagingWorkerDto = MessagingWorkerDto(),
    val channels: List<MessagingChannelDto> = emptyList(),
    val error: String? = null,
    @SerialName("management_path") val managementPath: String? = null
)

@Serializable
data class MessagingWorkerDto(
    val enabled: Boolean = false
)

@Serializable
data class MessagingChannelDto(
    @SerialName("connection_id") val connectionId: String,
    val kind: String,
    @SerialName("display_name") val displayName: String? = null,
    @SerialName("account_id") val accountId: String? = null,
    val enabled: Boolean = false,
    val state: String = "awaiting_verification",
    @SerialName("last_received_at") val lastReceivedAt: String? = null
)

@Serializable
data class ExecutionBackendDto(
    val id: String,
    @SerialName("display_name") val displayName: String,
    val availability: String,
    @SerialName("executable_path") val executablePath: String? = null,
    val version: String? = null,
    val capabilities: ExecutionBackendCapabilitiesDto
) {
    val availabilityState: ExecutionBackendAvailability
        get() = when (availability) {
            "available" -> ExecutionBackendAvailability.Available
            "not_installed" -> ExecutionBackendAvailability.NotInstalled
            "unauthenticated", "sign_in_required" -> ExecutionBackendAvailability.SignInRequired
            "incompatible", "incompatible_version" -> ExecutionBackendAvailability.Incompatible
            "unreachable" -> ExecutionBackendAvailability.Unreachable
            else -> ExecutionBackendAvailability.Unavailable
        }
}

enum class ExecutionBackendAvailability {
    Available,
    NotInstalled,
    SignInRequired,
    Incompatible,
    Unreachable,
    Unavailable,
}

@Serializable
data class ExecutionBackendCapabilitiesDto(
    @SerialName("streams_text") val streamsText: Boolean,
    @SerialName("streams_reasoning") val streamsReasoning: Boolean,
    @SerialName("streams_tools") val streamsTools: Boolean,
    @SerialName("supports_resume") val supportsResume: Boolean,
    @SerialName("supports_cancel") val supportsCancel: Boolean,
    @SerialName("supports_permission_bridge") val supportsPermissionBridge: Boolean
)

@Serializable
data class HostIdentityDto(
    val id: String,
    @SerialName("permission_protocol_version") val permissionProtocolVersion: Int
)

@Serializable
data class ComputerUseStatusDto(
    @SerialName("feature_enabled") val featureEnabled: Boolean = false,
    @SerialName("token_configured") val tokenConfigured: Boolean = false,
    val connected: Boolean = false,
    val hosts: List<ComputerUseHostDto>? = null,
    val tools: List<String>? = null
)

@Serializable
data class ComputerUseHostDto(
    @SerialName("host_id") val hostId: String,
    val capabilities: List<String>? = null,
    @SerialName("last_seen_age_sec") val lastSeenAgeSec: Double? = null
)

@Serializable
data class NativeBrowserStatusDto(
    @SerialName("token_configured") val tokenConfigured: Boolean = false,
    val connected: Boolean = false,
    @SerialName("structured_search") val structuredSearch: Boolean = false,
    val hosts: List<NativeBrowserHostDto>? = null,
    val tools: List<String>? = null,
    @SerialName("pending_commands") val pendingCommands: Int = 0,
    @SerialName("in_flight_commands") val inFlightCommands: Int = 0
)

@Serializable
data class NativeBrowserHostDto(
    @SerialName("host_id") val hostId: String,
    val capabilities: List<String>? = null,
    @SerialName("last_seen_age_sec") val lastSeenAgeSec: Double? = null
)

@Serializable
data class McpServerDto(
    val name: String = "",
    @SerialName("server_id") val serverId: String = "",
    val endpoint: String = "",
    val description: String? = null,
    val enabled: Boolean = true,
    val availability: String? = null,
    @SerialName("protocol_version") val protocolVersion: String? = null,
    val tools: Map<String, kotlinx.serialization.json.JsonElement> = emptyMap(),
    val resources: Map<String, kotlinx.serialization.json.JsonElement> = emptyMap(),
    @SerialName("resource_templates")
    val resourceTemplates: Map<String, kotlinx.serialization.json.JsonElement> = emptyMap(),
    val prompts: Map<String, kotlinx.serialization.json.JsonElement> = emptyMap()
) {
    val resolvedUrl: String
        get() = endpoint

    val resolvedName: String
        get() = name
}

@Serializable
data class UploadedContentDto(
    val name: String = "",
    val content: String? = null,
    val kind: String = "text",
    val enabled: Boolean = true,
    val description: String = "",
    val version: String? = null,
    @SerialName("required_tools") val requiredTools: List<String> = emptyList(),
    @SerialName("validation_steps") val validationSteps: List<String> = emptyList(),
) {
    val resolvedContent: String
        get() = content.orEmpty()
}

@Serializable
data class HostModelProfileDto(
    @SerialName("profile_id") val profileId: String,
    val revision: Long,
    @SerialName("display_name") val displayName: String,
    @SerialName("model_name") val modelName: String,
    val reasoning: HostModelReasoningDto? = null,
)

@Serializable
data class HostModelReasoningDto(val control: String, val efforts: List<String> = emptyList())

@Serializable
data class HostModelResolutionDto(
    val kind: String,
    @SerialName("profile_id") val profileId: String? = null,
)
