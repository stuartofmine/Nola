package com.nola.llm.provider

import android.content.Context
import com.nola.R
import com.nola.core.AgentCoreToolCatalog
import com.nola.core.AgentCoreRuntimeIdentity
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.remote.api.WebHostApiService
import com.nola.data.remote.api.normalizeWebHostBaseUrl
import com.nola.data.remote.api.rewriteLoopbackHttpUrlForWebHost
import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.providerMessages
import com.nola.llm.LlmClient
import com.nola.util.hostToolContainsSensitiveData
import com.nola.util.httpToolUriOrNull
import com.nola.util.redactHostToolText
import com.nola.util.rootRelativeToolPathOrNull
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.util.Locale
import java.util.TimeZone
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.time.Duration

sealed class WebAgentStreamEvent {
    data class Text(val content: String) : WebAgentStreamEvent()
    data class Reasoning(val content: String) : WebAgentStreamEvent()
    data class ToolUse(val event: HostAgentToolEvent) : WebAgentStreamEvent()
    data class ToolResult(
        val toolId: String,
        val toolName: String,
        val text: String,
        val display: String? = null,
        val error: Boolean = false
    ) : WebAgentStreamEvent()
    data class PermissionRequest(val request: HostAgentPermissionRequest) : WebAgentStreamEvent()
    data class Done(val reason: String = "stop") : WebAgentStreamEvent()
    data class Error(val message: String) : WebAgentStreamEvent()
}

fun interface WebAgentDeviceToolResolver {
    fun resolve(
        allowedCapabilities: Set<String>,
        identity: AgentCoreRuntimeIdentity,
        resultEgressAuthorized: Boolean,
        permissionMode: AndroidAgentPermissionMode,
    ): List<kotlinx.serialization.json.JsonElement>
}

@Singleton
class WebAgentProvider @Inject constructor(
    private val webHostApi: WebHostApiService,
    private val summaryTextSource: WebAgentSummaryTextSource
) : LlmClient {

    data class Config(
        val hostBaseUrl: String,
        val token: String,
        val timezone: String = TimeZone.getDefault().id,
        val language: String = Locale.getDefault().toLanguageTag(),
        val maxIterations: Int = 0,
        val nativeAppHostId: String = "",
        val allowLocalDevTools: Boolean = false,
        val expectedHostId: String = "",
        val clientId: String = "",
        val pinnedCertificateSha256: String = ""
    )

    var config: Config? = null
    private var frozenSummaryIntent:JsonObject? = null
    suspend fun freezeSummaryModel():WebAgentProvider {
        val captured=checkNotNull(config).copy()
        val intent=checkNotNull(fetchHostAgentSettings(captured)).freezeDefaultHostModelIntent(captured.expectedHostId)
        return WebAgentProvider(webHostApi,summaryTextSource).apply {config=captured;frozenSummaryIntent=intent}
    }
    internal fun frozenSummaryClient():Pair<LlmClient,String> {
        val captured=checkNotNull(config).copy()
        return WebAgentProvider(webHostApi,summaryTextSource).apply {config=captured} to captured.toString()
    }

    suspend fun goalControl(request: JsonObject, configuration: Config): JsonObject {
        require(request["expected_host_id"]?.jsonPrimitive?.content == configuration.expectedHostId)
        return webHostApi.goalControl(configuration.hostBaseUrl, configuration.token, request,
            configuration.pinnedCertificateSha256)
    }

    suspend fun conversationProgress(conversationId: String, configuration: Config): com.nola.data.repository.ConversationProgress {
        require(configuration.expectedHostId.isNotBlank() && conversationId.isNotBlank())
        val response = webHostApi.conversationProgress(configuration.hostBaseUrl, configuration.token,
            buildJsonObject { put("expected_host_id", configuration.expectedHostId); put("conversation_id", conversationId) },
            configuration.pinnedCertificateSha256)
        return com.nola.data.repository.ConversationProgressRepository.decodeRemote(
            conversationId, configuration.expectedHostId, response)
    }

    suspend fun submissionFrontier(conversationId: String, owner: com.nola.data.remote.api.RemoteRunKey?,
        displayedParentMessageId: String?, configuration: Config): com.nola.data.remote.api.RemoteSubmissionFrontier {
        val request = com.nola.data.remote.api.remoteSubmissionFrontierRequest(
            configuration.expectedHostId, configuration.clientId, conversationId, owner)
        val response = webHostApi.submissionFrontier(configuration.hostBaseUrl, configuration.token, request,
            configuration.pinnedCertificateSha256)
        return com.nola.data.remote.api.decodeRemoteSubmissionFrontier(response, configuration.expectedHostId,
            configuration.clientId, conversationId, displayedParentMessageId)
    }

    suspend fun permissionConfiguration(owner: com.nola.data.remote.api.RemoteRunKey, configuration: Config,
        expectedRevision: Long? = null, profile: AndroidAgentPermissionMode? = null, minimumPolicyRevision: Long? = null): JsonObject {
        require(configuration.expectedHostId.isNotBlank() && configuration.clientId.isNotBlank() &&
            (expectedRevision == null || expectedRevision in 0 until Long.MAX_VALUE) &&
            (expectedRevision == null) == (profile == null))
        val response = webHostApi.permissionConfiguration(configuration.hostBaseUrl, configuration.token, buildJsonObject {
            put("operation", if (profile == null) "read" else "patch")
            put("expected_host_id", configuration.expectedHostId); put("client_id", configuration.clientId)
            put("conversation_id", owner.conversationId); put("expected_run_id", owner.runId)
            put("assistant_message_id", owner.assistantMessageId)
            minimumPolicyRevision?.let { put("minimum_policy_revision", it) }
            expectedRevision?.let { put("expected_revision", it) }; profile?.let { put("permission_profile", it.wireValue) }
        }, configuration.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == configuration.expectedHostId)
        val result = response["result"] as? JsonObject ?: error("Host permission configuration is missing")
        require(result.keys == setOf("conversation_id", "configuration_revision", "permission_profile", "settings", "owner", "state", "active_run_id", "active_policy"))
        require(result["conversation_id"]?.jsonPrimitive?.content == owner.conversationId)
        val revision = result["configuration_revision"]?.jsonPrimitive?.content?.toLongOrNull() ?: error("Host configuration revision is missing")
        require(revision > 0)
        require(result["permission_profile"]?.jsonPrimitive?.content in setOf("null", "cautious", "allow_all"))
        val activeRun = result["active_run_id"]?.jsonPrimitive?.content?.takeUnless { it == "null" }
        require(activeRun == null || activeRun == owner.runId)
        require(result["state"]?.jsonPrimitive?.content in setOf("inactive", "pending", "active"))
        val permissionOwner = result["owner"] as? JsonObject ?: error("Permission owner is missing")
        require(permissionOwner["kind"]?.jsonPrimitive?.content in setOf("inactive", "local", "remote"))
        when (permissionOwner["kind"]?.jsonPrimitive?.content) {
            "inactive" -> require(activeRun == null && result["state"]?.jsonPrimitive?.content == "inactive")
            "local" -> require(permissionOwner["run_id"]?.jsonPrimitive?.content == activeRun && activeRun == owner.runId &&
                permissionOwner["assistant_message_id"]?.jsonPrimitive?.content == owner.assistantMessageId)
            "remote" -> {
                val identity = permissionOwner["identity"] as? JsonObject ?: error("Remote permission identity is missing")
                require(identity["source_host_id"]?.jsonPrimitive?.content == configuration.expectedHostId &&
                    identity["client_id"]?.jsonPrimitive?.content == configuration.clientId &&
                    identity["conversation_id"]?.jsonPrimitive?.content == owner.conversationId &&
                    identity["submission_id"]?.jsonPrimitive?.content == owner.runId && activeRun == owner.runId &&
                    identity["assistant_message_id"]?.jsonPrimitive?.content == owner.assistantMessageId)
            }
        }
        val policy = result["active_policy"] as? JsonObject
        require(policy != null || result["active_policy"] == kotlinx.serialization.json.JsonNull)
        if (policy != null) require(policy["run_id"]?.jsonPrimitive?.content == owner.runId && activeRun == owner.runId &&
            (policy["configuration_revision"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0) in 1..revision &&
            policy["profile"]?.jsonPrimitive?.content in setOf("cautious", "allow_all"))
        if (profile != null) require(result["permission_profile"]?.jsonPrimitive?.content == profile.wireValue && revision == checkNotNull(expectedRevision) + 1)
        return result
    }

    suspend fun idleUiQueueHead(conversationId: String, configuration: Config, expectedSourceStoreEpoch: String): com.nola.data.remote.api.RemoteUiQueueState {
        require(expectedSourceStoreEpoch.isNotBlank())
        val captured = configuration.copy()
        require(captured.expectedHostId.isNotBlank() && captured.clientId.isNotBlank())
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "queued-input", buildJsonObject {
            put("expected_source_store_epoch", expectedSourceStoreEpoch)
            put("expected_host_id", captured.expectedHostId); put("conversation_id", conversationId); put("client_id", captured.clientId)
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        return com.nola.data.remote.api.decodeRemoteUiQueue(response.getValue("result").jsonObject,
            captured.expectedHostId, captured.clientId, conversationId)
    }

    suspend fun cancelQueuedUiInput(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt, configuration: Config, expectedSourceStoreEpoch: String) {
        require(expectedSourceStoreEpoch.isNotBlank())
        val captured = configuration.copy()
        require(receipt.identity.sourceHostId == captured.expectedHostId && receipt.identity.clientId == captured.clientId)
        require(receipt.phase in setOf(com.nola.data.remote.api.RemoteSubmissionPhase.Rejected, com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled))
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "queued-input/cancel", buildJsonObject {
            put("expected_source_store_epoch", expectedSourceStoreEpoch)
            put("expected_host_id", captured.expectedHostId); put("client_id", captured.clientId)
            put("conversation_id", receipt.identity.conversationId); put("command_id", receipt.identity.submissionId)
            put("request_digest", receipt.requestDigest)
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        val result = response.getValue("result").jsonObject
        require(result["cancelled"]?.jsonPrimitive?.boolean == true && result["command_id"]?.jsonPrimitive?.content == receipt.identity.submissionId)
    }

    suspend fun listUiSurfaces(conversationId: String, configuration: Config, expectedSourceStoreEpoch: String): List<com.nola.data.local.UiSnapshot> {
        require(expectedSourceStoreEpoch.isNotBlank())
        val captured = configuration.copy()
        require(captured.expectedHostId.isNotBlank())
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "surfaces", buildJsonObject {
            put("expected_source_store_epoch", expectedSourceStoreEpoch)
            put("expected_host_id", captured.expectedHostId); put("conversation_id", conversationId)
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        val surfaces = Json.decodeFromString<List<com.nola.data.local.UiSnapshot>>(response.getValue("result").toString())
        require(surfaces.size <= 8 && surfaces.all { it.identity.owner.conversation_id == conversationId && it.identity.owner.authority_id == expectedSourceStoreEpoch })
        return surfaces
    }

    suspend fun readUiSurfaceChanges(identity: com.nola.data.local.UiSurfaceIdentity, baseRevision: Long?, configuration: Config): com.nola.data.local.UiSurfaceUpdate {
        val captured = configuration.copy()
        require(captured.expectedHostId.isNotBlank())
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "changes", buildJsonObject {
            put("expected_host_id", captured.expectedHostId)
            put("identity", Json.parseToJsonElement(Json.encodeToString(identity)))
            put("base_revision", baseRevision?.let(::JsonPrimitive) ?: JsonNull)
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        return Json.decodeFromString(response.getValue("result").toString())
    }

    suspend fun readUiSurfaceRevision(reference: com.nola.data.local.UiSurfaceRevisionRef, configuration: Config): com.nola.data.local.UiSnapshot? {
        val captured = configuration.copy()
        require(captured.expectedHostId.isNotBlank() && reference.revision > 0)
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "revision", buildJsonObject {
            put("expected_host_id", captured.expectedHostId)
            put("identity", Json.parseToJsonElement(Json.encodeToString(reference.identity)))
            put("revision", reference.revision)
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        val snapshot = Json.decodeFromString<com.nola.data.local.UiSnapshot?>(response.getValue("result").toString())
        require(snapshot == null || snapshot.identity == reference.identity && snapshot.revision == reference.revision &&
            snapshot.phase != com.nola.data.local.UiSurfacePhase.Ready && snapshot.nodes.values.all { it.actions.isEmpty() && it.bindings.isEmpty() })
        return snapshot
    }

    suspend fun previewUiSurface(input: com.nola.data.local.UiPreviewInput, configuration: Config): com.nola.data.local.UiPreview {
        val captured = configuration.copy()
        val fields = kotlinx.serialization.json.Json.encodeToJsonElement(com.nola.data.local.UiPreviewInput.serializer(), input).jsonObject
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "preview",
            JsonObject(fields + ("expected_host_id" to JsonPrimitive(captured.expectedHostId))), captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        return kotlinx.serialization.json.Json.decodeFromJsonElement(com.nola.data.local.UiPreview.serializer(), response.getValue("result"))
    }

    suspend fun submitUiInteraction(input: com.nola.data.local.UiSubmission, configuration: Config): com.nola.data.local.UiRemoteInteractionResult {
        val captured = configuration.copy()
        require(captured.expectedHostId.isNotBlank())
        val response = webHostApi.generativeUi(captured.hostBaseUrl, captured.token, "interactions", buildJsonObject {
            put("expected_host_id", captured.expectedHostId)
            put("input", Json.parseToJsonElement(Json.encodeToString(input)))
        }, captured.pinnedCertificateSha256)
        require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId)
        val result = Json.decodeFromString<com.nola.data.local.UiRemoteInteractionResult>(response.getValue("result").toString())
        require(result.status in setOf("committed", "rejected", "conflict", "unknown"))
        if (result.status == "committed") {
            val receipt = checkNotNull(result.receipt)
            require(receipt.snapshot.identity == input.interaction.identity &&
                receipt.snapshot.revision > input.interaction.expected_surface_revision &&
                receipt.snapshot.revision - input.interaction.expected_surface_revision == 1L &&
                (receipt.queued_submission_id == null || receipt.queued_submission_id == input.submission_id))
        }
        return result
    }

    /** Freeze pairing, token and TLS pin for the complete transfer. */
    fun journalTransport(configuration: Config? = config): suspend (JsonObject) -> JsonObject {
        val captured = checkNotNull(configuration) { "Web Agent Host is not configured" }.copy()
        require(captured.expectedHostId.isNotBlank())
        return { request ->
            val response = webHostApi.readAssistantJournal(captured.hostBaseUrl, captured.token, request, captured.pinnedCertificateSha256)
            require(response["source_host_id"]?.jsonPrimitive?.content == captured.expectedHostId) { "Assistant journal source mismatch" }
            response
        }
    }

    suspend fun expertMessageRequest(operation: String, input: JsonObject, configuration: Config): JsonObject {
        require(operation in setOf("parent", "admit", "manifest") && configuration.expectedHostId.isNotBlank() && configuration.clientId.isNotBlank())
        require(input.keys.none { it in setOf("expected_host_id", "client_id", "authority", "actor", "builtins", "context") })
        val response = webHostApi.expertMessageCommand(configuration.hostBaseUrl, configuration.token, operation,
            buildJsonObject { put("expected_host_id", configuration.expectedHostId); put("client_id", configuration.clientId); input.forEach { (key,value) -> put(key,value) } }, configuration.pinnedCertificateSha256)
        require(response["host_id"]?.jsonPrimitive?.content == configuration.expectedHostId &&
            response["client_id"]?.jsonPrimitive?.content == configuration.clientId) { "Expert message source changed" }
        return response
    }

    suspend fun expertRoomReply(command: JsonObject, configuration: Config, expectedStoreEpoch: String?): JsonObject {
        require(command["type"]?.jsonPrimitive?.content in setOf("directory", "role_templates", "read", "for_conversation", "ensure_for_conversation", "join", "rejoin", "configure_agent", "submission", "submissions", "child_conversations", "child_history"))
        require(configuration.expectedHostId.isNotBlank())
        if (expectedStoreEpoch == null) require(command["type"]?.jsonPrimitive?.content in setOf("directory", "role_templates", "read", "for_conversation", "submission", "submissions", "child_conversations", "child_history")) { "Read the source catalog before changing experts" }
        val response = webHostApi.agentRoomCommand(configuration.hostBaseUrl, configuration.token,
            buildJsonObject { put("expected_host_id", configuration.expectedHostId); put("command", command); expectedStoreEpoch?.let { put("expected_source_store_epoch", it) } }, configuration.pinnedCertificateSha256)
        require(response["host_id"]?.jsonPrimitive?.content == configuration.expectedHostId) { "Expert Room Host changed" }
        val epoch = response["source_store_epoch"]?.jsonPrimitive?.content?.takeIf { it.isNotBlank() }
            ?: error("The Host does not support source-bound expert operations; update or reconnect it")
        require(expectedStoreEpoch == null || epoch == expectedStoreEpoch) { "The expert source Store changed" }
        return response
    }

    suspend fun expertRoomCommand(command: JsonObject, configuration: Config, expectedStoreEpoch: String? = null): JsonElement =
        expertRoomReply(command, configuration, expectedStoreEpoch).getValue("result")

    /** Read only Room history through the captured, encrypted Host connection. */
    suspend fun readExpertHistoryCommand(command: JsonObject, configuration: Config, expectedStoreEpoch: String? = null): JsonObject {
        require(command["type"]?.jsonPrimitive?.content in setOf("child_conversations", "child_history", "for_conversation"))
        require(configuration.expectedHostId.isNotBlank())
        return expertRoomCommand(command, configuration, expectedStoreEpoch).jsonObject
    }

    fun observationTransport(configuration: Config? = config): suspend (JsonObject) -> JsonObject {
        val captured = checkNotNull(configuration).copy()
        require(captured.expectedHostId.isNotBlank() && captured.clientId.isNotBlank())
        return { request -> webHostApi.observeRemoteConversation(captured.hostBaseUrl, captured.token,
            request, captured.pinnedCertificateSha256) }
    }

    fun resumeTransport(configuration: Config? = config): (com.nola.data.remote.api.RemoteRunKey) -> Flow<SseEvent> {
        val captured = checkNotNull(configuration).copy()
        require(captured.expectedHostId.isNotBlank() && captured.clientId.isNotBlank())
        return { key -> webHostApi.resumeRemoteConversation(captured.hostBaseUrl, captured.token,
            com.nola.data.remote.api.remoteResumeRequest(captured.expectedHostId, captured.clientId, key),
            captured.pinnedCertificateSha256) }
    }

    suspend fun fetchHostAgentSettings(configuration: Config): AgentSettingsResponse? {
        val cfg = configuration
        return webHostApi.getAgentSettings(
            hostBaseUrl = cfg.hostBaseUrl,
            token = cfg.token,
            pinnedCertificateSha256 = cfg.pinnedCertificateSha256
        )
    }

    suspend fun submitHostPermissionDecision(payload: Map<String, Any>, configuration: Config? = config) {
        val cfg = configuration ?: error("Web Agent Host is not configured")
        webHostApi.submitAgentPermissionDecision(
            hostBaseUrl = cfg.hostBaseUrl,
            token = cfg.token,
            payload = payload,
            pinnedCertificateSha256 = cfg.pinnedCertificateSha256
        )
    }

    suspend fun cancelAdmittedSubmission(identity: com.nola.data.remote.api.RemoteSubmissionIdentity,
        requestDigest: String, configuration: Config? = config) {
        val cfg = configuration ?: error("Web Agent Host is not configured")
        require(identity.sourceHostId == cfg.expectedHostId && identity.clientId == cfg.clientId)
        webHostApi.cancelAdmittedRemoteSubmission(
            hostBaseUrl = cfg.hostBaseUrl,
            token = cfg.token,
            identity = identity, requestDigest = requestDigest,
            pinnedCertificateSha256 = cfg.pinnedCertificateSha256
        )
    }

    suspend fun cancelAgentTask(conversationId: String, parentRunId: String, taskId: String, configuration: Config): JsonObject {
        val intent = buildJsonObject {
            put("hostId", configuration.expectedHostId); put("conversationId", conversationId)
            put("parentRunId", parentRunId); put("taskId", taskId)
        }
        return com.nola.data.local.requestRemoteTaskCancellation(intent) { receiptOnly ->
            webHostApi.taskCancellationCommand(configuration.hostBaseUrl, configuration.token, intent,
                receiptOnly, configuration.pinnedCertificateSha256)
        }
    }

    suspend fun taskGuidanceCommand(intent: JsonObject, receiptOnly: Boolean, configuration: Config): JsonObject =
        webHostApi.taskGuidanceCommand(configuration.hostBaseUrl, configuration.token, intent,
            receiptOnly, configuration.pinnedCertificateSha256)

    fun taskProjectionEvents(conversationId: String, afterCursor: Long, configuration: Config): Flow<SseEvent> {
        val cfg = configuration
        return webHostApi.streamAgentTaskEvents(
            hostBaseUrl = cfg.hostBaseUrl,
            token = cfg.token,
            conversationId = conversationId,
            afterCursor = afterCursor,
            pinnedCertificateSha256 = cfg.pinnedCertificateSha256,
        )
    }

    suspend fun submitDeviceToolResult(payload: Map<String, Any>, configuration: Config? = config) {
        val cfg = configuration ?: error("Web Agent Host is not configured")
        require(payload["host_id"] == cfg.expectedHostId && payload["client_id"] == cfg.clientId)
        webHostApi.submitAgentDeviceToolResult(
            hostBaseUrl = cfg.hostBaseUrl,
            token = cfg.token,
            payload = payload,
            pinnedCertificateSha256 = cfg.pinnedCertificateSha256
        )
    }

    suspend fun freezeHostToolPlan(request: JsonObject, configuration: Config? = config): JsonObject {
        val cfg = checkNotNull(configuration)
        val sessionId = request["conversationId"]?.jsonPrimitive?.content ?: error("Missing conversation identity")
        require(sessionId.isNotBlank() && request["tool_plan_ref"] == null)
        require(request["model_intent"] == null) { "Submission model intent is already frozen" }
        val modelSettings = if (request["execution_backend"]?.jsonPrimitive?.content == "nola") {
            checkNotNull(fetchHostAgentSettings(cfg))
        } else null
        val modelIntent = frozenSummaryIntent ?: modelSettings?.freezeDefaultHostModelIntent(cfg.expectedHostId)
        val response = webHostApi.prepareToolPlan(cfg.hostBaseUrl, cfg.token, buildJsonObject {
            put("expected_host_id", cfg.expectedHostId); put("client_id", cfg.clientId); put("session_id", sessionId)
        }, cfg.pinnedCertificateSha256)
        val reference = com.nola.data.remote.api.validatedHostToolPlanReference(response, cfg.expectedHostId, cfg.clientId, sessionId)
        if (modelSettings != null && frozenSummaryIntent == null) {
            modelSettings.requireUnchangedHostDefault(checkNotNull(fetchHostAgentSettings(cfg)), cfg.expectedHostId)
        }
        return JsonObject(request + ("tool_plan_ref" to reference) +
            (modelIntent?.let { mapOf("model_intent" to it) } ?: emptyMap()))
    }

    fun prepareAgentRequest(
        input: ModelInput,
        conversationId: String? = null,
        agentMode: Boolean = true,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
        responseStyle: String = "casual",
        customSystemPrompt: String = "",
        nativeAppHostId: String = "",
        hybridDeviceToolsEnabled: Boolean = false,
        hybridDeviceCapabilities: Set<String> = emptySet(),
        executionBackend: String = "nola",
        externalSessionId: String = "",
        submissionId: String,
        currentUserDisplayText: String? = null,
        configuration: Config? = config,
        expectedConfigurationRevision: Long? = null,
        expectedParentMessageId: String? = null,
        queuedUserCommandId: String? = null,
        selectedExperts: JsonArray? = null,
        expertSourceEpoch: String? = null,
    ): JsonObject {
        val cfg = checkNotNull(configuration)
        require(selectedExperts.isNullOrEmpty() || !expertSourceEpoch.isNullOrBlank()) { "Expert directory source is unavailable" }
        require(cfg.expectedHostId.isNotBlank() && cfg.clientId.isNotBlank())
        val preferredNativeAppHostId = nativeAppHostId.ifBlank { cfg.nativeAppHostId }
        val deviceBridge = if (hybridDeviceToolsEnabled) {
            deviceBridgeAdvertisement(cfg, conversationId, hybridDeviceCapabilities, input.tools)
        } else null

        val messages = input.providerMessages()

        val body = com.nola.data.remote.api.buildWebHostAgentRequestBody(
            messages = messages,
            conversationId = conversationId,
            allowLocalDevTools = cfg.allowLocalDevTools,
            maxIterations = cfg.maxIterations,
            timezone = cfg.timezone,
            language = cfg.language,
            agentMode = agentMode,
            permissionProfile = permissionMode,
            responseStyle = responseStyle,
            customSystemPrompt = customSystemPrompt,
            nativeAppHostId = preferredNativeAppHostId,
            expectedHostId = cfg.expectedHostId,
            clientId = cfg.clientId,
            deviceBridge = deviceBridge,
            executionBackend = executionBackend,
            externalSessionId = externalSessionId,
            submissionId = submissionId,
            queuedUserCommandId = queuedUserCommandId,
        )
        val request = JsonObject(com.nola.data.remote.mobile_crypto.MobileCrypto.payloadJsonObject(body) + mapOf(
            "expected_configuration_revision" to (expectedConfigurationRevision?.let(::JsonPrimitive) ?: kotlinx.serialization.json.JsonNull),
            "expected_parent_message_id" to (expectedParentMessageId?.let(::JsonPrimitive) ?: kotlinx.serialization.json.JsonNull),
        ))
        return JsonObject(request + (currentUserDisplayText?.let { mapOf("current_user_display_text" to JsonPrimitive(it)) } ?: emptyMap()) +
            (selectedExperts?.takeIf { it.isNotEmpty() }?.let { mapOf("selected_experts" to it, "expected_source_store_epoch" to JsonPrimitive(checkNotNull(expertSourceEpoch))) } ?: emptyMap()))
    }

    fun streamFrozenSubmission(submission: com.nola.data.remote.api.FrozenRemoteSubmission,
        configuration: Config? = config): Flow<SseEvent> {
        val cfg = checkNotNull(configuration).copy()
        require(submission.identity.sourceHostId == cfg.expectedHostId && submission.identity.clientId == cfg.clientId)
        return webHostApi.streamBoundAgentRequest(cfg.hostBaseUrl, cfg.token, submission, cfg.pinnedCertificateSha256)
    }

    fun submissionTransport(configuration: Config? = config): suspend (JsonObject) -> JsonObject {
        val cfg = checkNotNull(configuration).copy()
        return { request -> webHostApi.remoteSubmission(cfg.hostBaseUrl, cfg.token, request, cfg.pinnedCertificateSha256).also {
            require(it["source_host_id"]?.jsonPrimitive?.content == cfg.expectedHostId)
        } }
    }

    internal var deviceToolResolver = WebAgentDeviceToolResolver { allowedCapabilities, identity, authorized, permissionMode ->
        AgentCoreToolCatalog.hybridAdvertisementTools(
            allowedCapabilities,
            identity,
            authorized,
            permissionMode,
        )
    }

    fun contextToolSchemas(
        conversationId: String?,
        hybridDeviceToolsEnabled: Boolean,
        hybridDeviceCapabilities: Set<String>,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
    ): List<JsonElement> {
        if (!hybridDeviceToolsEnabled) return emptyList()
        val cfg = config ?: return emptyList()
        if (cfg.expectedHostId.isBlank() || cfg.clientId.isBlank() || conversationId.isNullOrBlank()) return emptyList()
        return deviceToolResolver.resolve(
            hybridDeviceCapabilities,
            AgentCoreRuntimeIdentity(cfg.expectedHostId, cfg.clientId, conversationId),
            true,
            permissionMode,
        )
    }

    private fun deviceBridgeAdvertisement(
        config: Config,
        sessionId: String?,
        allowedCapabilities: Set<String>,
        preparedTools: List<JsonElement>,
    ): JsonObject? {
        if (config.expectedHostId.isBlank() || config.clientId.isBlank() || sessionId.isNullOrBlank()) return null
        val tools = preparedTools.mapNotNull { schema ->
            val root = schema as? JsonObject ?: return@mapNotNull null
            val metadata = root["x_nola"] as? JsonObject ?: return@mapNotNull null
            if (metadata["target"]?.jsonPrimitive?.content != "device") return@mapNotNull null
            val capability = metadata["capability"]?.jsonPrimitive?.content ?: return@mapNotNull null
            if (capability !in allowedCapabilities) return@mapNotNull null
            buildJsonObject {
                put("schema", root)
                put("capability", JsonPrimitive(capability))
                put("allow_result_egress", JsonPrimitive(true))
            }
        }
        if (tools.isEmpty()) return null
        return buildJsonObject {
            put("protocol_version", JsonPrimitive(1))
            put("host_id", JsonPrimitive(config.expectedHostId))
            put("client_id", JsonPrimitive(config.clientId))
            put("session_id", JsonPrimitive(sessionId))
            put("tools", JsonArray(tools))
        }
    }

    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        val builder = StringBuilder()
        generateSummaryStream(input).collect { builder.append(it) }
        return ModelOutput(builder.toString().trim())
    }

    override fun generateSummaryStream(input: ModelInput): Flow<String> = flow {
        val cfg = checkNotNull(config) { "Web Agent Host is not configured" }
        val conversation = java.util.UUID.randomUUID().toString()
        val submission = java.util.UUID.randomUUID().toString()
        val identity = com.nola.data.remote.api.RemoteSubmissionIdentity(cfg.expectedHostId, cfg.clientId,
            conversation, submission, submission, "$submission:assistant")
        val request = freezeHostToolPlan(prepareAgentRequest(input, conversationId = conversation, submissionId = submission, configuration = cfg), cfg)
        val submit = submissionTransport(cfg)
        // Auxiliary summaries need no local replica intent or fake conversation.
        // The source-only digest operation is not an admission receipt.
        val prepared = submit(buildJsonObject { put("operation", JsonPrimitive("prepare")); put("identity", identity.wire()); put("request", request) })
            .getValue("result") as? JsonObject ?: error("Missing source digest preparation")
        require(prepared["identity"] == identity.wire())
        val frozen = com.nola.data.remote.api.FrozenRemoteSubmission(identity, prepared.getValue("request_digest").jsonPrimitive.content, request)
        val reader = RemoteJournalSummary(cfg.expectedHostId, conversation, submission, journalTransport(cfg))
        var committed: String? = null
        var streamFailure: Exception? = null
        try { streamFrozenSubmission(frozen, cfg).collect { event ->
            if (committed != null) return@collect
            when (event) {
                is SseEvent.SubmissionReceipt -> reader.acceptReceipt(com.nola.data.remote.api.decodeRemoteSubmissionReceipt(
                    event.receipt, identity, frozen.requestDigest))
                is SseEvent.AssistantJournal -> reader.observe(event.envelope)
                is SseEvent.AssistantTerminal -> committed = reader.terminal(event.envelope)
                is SseEvent.PermissionRequest -> error("Summary requires interactive permission")
                is SseEvent.DeviceToolRequest -> error("Summary requires device approval")
                is SseEvent.Error -> error(redactHostToolText(event.message))
                else -> Unit // Trace, lifecycle and task events are not answer/terminal authority.
            }
        } } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (failure: Exception) { streamFailure = failure }
        if (!reader.isAdmitted()) {
            val result = try { submit(frozen.reconciliationRequest()).getValue("result") }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) { throw com.nola.data.remote.api.RemoteSubmissionUnresolved(frozen, failure) }
            if (result is JsonObject) reader.acceptReceipt(com.nola.data.remote.api.decodeRemoteSubmissionReceipt(result, identity, frozen.requestDigest))
        }
        val answer = committed ?: reader.terminal()
        if (answer == null) throw com.nola.data.remote.api.RemoteSubmissionUnresolved(frozen, streamFailure)
        emit(answer)
    }

    override suspend fun chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Boolean,
        timeout: Duration?
    ): String {
        val input = ModelInput(
            messages = listOf(mapOf("role" to "user", "content" to message)),
            systemPrompt = systemPrompt,
            userPrefs = mapOf("enable_thinking" to enableThinking.toString())
        )
        return generateSummary(input, timeout).text
    }
}

interface WebAgentSummaryTextSource {
    fun current(): WebAgentSummaryText
}

object LocaleWebAgentSummaryTextSource : WebAgentSummaryTextSource {
    override fun current(): WebAgentSummaryText = WebAgentSummaryText.forLocale()
}

@Singleton
class ResourceWebAgentSummaryTextSource @Inject constructor(
    @ApplicationContext private val context: Context
) : WebAgentSummaryTextSource {
    override fun current(): WebAgentSummaryText {
        return WebAgentSummaryText(
            webHostNotConfigured = context.getString(R.string.chat_web_agent_summary_web_host_not_configured),
            toolRunningFormat = context.getString(R.string.chat_web_agent_summary_tool_running_format),
            toolResultFormat = context.getString(R.string.chat_web_agent_summary_tool_result_format),
            sensitiveToolResultHidden = context.getString(R.string.chat_web_agent_summary_sensitive_hidden),
            done = context.getString(R.string.chat_web_agent_summary_done),
            permissionRequiredFallback = context.getString(R.string.chat_web_agent_summary_permission_required_fallback),
            permissionRequiredFormat = context.getString(R.string.chat_web_agent_summary_permission_required_format),
            errorFormat = context.getString(R.string.chat_web_agent_summary_error_format)
        )
    }
}

data class WebAgentSummaryText(
    val webHostNotConfigured: String,
    val toolRunningFormat: String,
    val toolResultFormat: String,
    val sensitiveToolResultHidden: String,
    val done: String,
    val permissionRequiredFallback: String,
    val permissionRequiredFormat: String,
    val errorFormat: String
) {
    fun toolRunningLine(toolName: String): String = formatted(toolRunningFormat, toolName)

    fun toolResultLine(toolName: String, summary: String): String = formatted(toolResultFormat, toolName, summary)

    fun permissionRequiredLine(toolName: String, summary: String): String =
        formatted(permissionRequiredFormat, toolName, summary)

    fun errorLine(message: String): String = formatted(errorFormat, message)

    private fun formatted(format: String, vararg args: Any): String =
        String.format(Locale.getDefault(), format, *args)

    companion object {
        fun forLocale(locale: Locale = Locale.getDefault()): WebAgentSummaryText {
            return forLanguage(locale.language)
        }

        fun forLanguage(language: String?): WebAgentSummaryText {
            return when (language?.lowercase(Locale.ROOT)) {
                "zh" -> WebAgentSummaryText(
                    webHostNotConfigured = "Web 宿主机尚未配置。",
                    toolRunningFormat = "\n\n🔧 %s 正在运行…\n",
                    toolResultFormat = "\n✅ %s：%s\n",
                    sensitiveToolResultHidden = "敏感工具结果已隐藏",
                    done = "完成",
                    permissionRequiredFallback = "需要权限确认",
                    permissionRequiredFormat = "\n⚠️ %s 需要权限确认：%s\n",
                    errorFormat = "\n错误：%s\n"
                )
                "de" -> WebAgentSummaryText(
                    webHostNotConfigured = "Web-Host ist nicht konfiguriert.",
                    toolRunningFormat = "\n\n🔧 %s wird ausgeführt…\n",
                    toolResultFormat = "\n✅ %s: %s\n",
                    sensitiveToolResultHidden = "Sensibles Tool-Ergebnis ausgeblendet",
                    done = "Fertig",
                    permissionRequiredFallback = "Berechtigung erforderlich",
                    permissionRequiredFormat = "\n⚠️ %s erfordert eine Berechtigung: %s\n",
                    errorFormat = "\nFehler: %s\n"
                )
                "es" -> WebAgentSummaryText(
                    webHostNotConfigured = "El host web no está configurado.",
                    toolRunningFormat = "\n\n🔧 %s está en ejecución…\n",
                    toolResultFormat = "\n✅ %s: %s\n",
                    sensitiveToolResultHidden = "Resultado sensible de la herramienta oculto",
                    done = "Listo",
                    permissionRequiredFallback = "Permiso requerido",
                    permissionRequiredFormat = "\n⚠️ %s requiere permiso: %s\n",
                    errorFormat = "\nError: %s\n"
                )
                "fr" -> WebAgentSummaryText(
                    webHostNotConfigured = "L'hôte Web n'est pas configuré.",
                    toolRunningFormat = "\n\n🔧 %s est en cours…\n",
                    toolResultFormat = "\n✅ %s : %s\n",
                    sensitiveToolResultHidden = "Résultat sensible de l'outil masqué",
                    done = "Terminé",
                    permissionRequiredFallback = "Autorisation requise",
                    permissionRequiredFormat = "\n⚠️ %s nécessite une autorisation : %s\n",
                    errorFormat = "\nErreur : %s\n"
                )
                "ja" -> WebAgentSummaryText(
                    webHostNotConfigured = "Web ホストが設定されていません。",
                    toolRunningFormat = "\n\n🔧 %s を実行中…\n",
                    toolResultFormat = "\n✅ %s: %s\n",
                    sensitiveToolResultHidden = "機密性の高いツール結果は非表示です",
                    done = "完了",
                    permissionRequiredFallback = "権限確認が必要です",
                    permissionRequiredFormat = "\n⚠️ %s には権限の確認が必要です: %s\n",
                    errorFormat = "\nエラー: %s\n"
                )
                "ru" -> WebAgentSummaryText(
                    webHostNotConfigured = "Web-хост не настроен.",
                    toolRunningFormat = "\n\n🔧 %s выполняется…\n",
                    toolResultFormat = "\n✅ %s: %s\n",
                    sensitiveToolResultHidden = "Чувствительный результат инструмента скрыт",
                    done = "Готово",
                    permissionRequiredFallback = "Требуется разрешение",
                    permissionRequiredFormat = "\n⚠️ %s требует разрешения: %s\n",
                    errorFormat = "\nОшибка: %s\n"
                )
                else -> WebAgentSummaryText(
                    webHostNotConfigured = "Web host is not configured.",
                    toolRunningFormat = "\n\n🔧 %s is running…\n",
                    toolResultFormat = "\n✅ %s: %s\n",
                    sensitiveToolResultHidden = "Sensitive tool result hidden",
                    done = "Done",
                    permissionRequiredFallback = "Permission required",
                    permissionRequiredFormat = "\n⚠️ %s requires permission: %s\n",
                    errorFormat = "\nError: %s\n"
                )
            }
        }
    }
}

internal fun summaryStreamPermissionLine(
    event: SseEvent.PermissionRequest,
    text: WebAgentSummaryText = WebAgentSummaryText.forLocale()
): String {
    val rawSummary = event.message.ifBlank { event.reason }
    val summary = redactHostToolText(rawSummary).trim().ifBlank { text.permissionRequiredFallback }
    return text.permissionRequiredLine(event.toolName, summary)
}

internal fun resolveAgainstHostBase(raw: String?, hostBaseUrl: String): String? {
    httpToolUriOrNull(raw)?.let { return rewriteLoopbackHttpUrlForWebHost(it, hostBaseUrl) }

    val path = rootRelativeToolPathOrNull(raw) ?: return null
    val base = normalizeWebHostBaseUrl(hostBaseUrl).ifBlank { return null }
    return "$base$path"
}

private fun hostToolMetaFlag(meta: Map<String, String>, key: String): Boolean {
    return meta[key]?.trim()?.lowercase() in setOf("true", "1", "yes", "y", "on")
}
