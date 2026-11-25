package com.nola.data.remote.api

import com.nola.core.AndroidAgentPermissionMode

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.decodeAgentCoreTaskProjection
import com.nola.data.remote.dto.VerifiedHostTransport
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.delete
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.patch
import io.ktor.client.request.post
import io.ktor.client.request.preparePost
import io.ktor.client.statement.bodyAsChannel
import io.ktor.utils.io.readRemaining
import io.ktor.utils.io.core.readBytes
import kotlinx.serialization.decodeFromString
import io.ktor.client.request.setBody
import io.ktor.http.ContentType
import io.ktor.http.content.TextContent
import io.ktor.http.contentType
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.decodeFromJsonElement
import java.net.URI
import java.security.MessageDigest
import java.util.Base64
import java.util.UUID
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import javax.inject.Inject
import javax.inject.Singleton

internal val GENERATIVE_UI_OPERATIONS: Set<String> = setOf(
    "surfaces",
    "interactions",
    "queued-input",
    "queued-input/cancel",
    "revision",
    "changes",
    "preview",
)

internal fun generativeUiMaxResponseBytes(operation: String): Int = when (operation) {
    "surfaces" -> 16 * 1024 * 1024
    "interactions", "revision", "changes", "preview" -> 2 * 1024 * 1024
    else -> 64 * 1024
}

@Singleton
class WebHostApiServiceImpl @Inject constructor(
    private val httpClient: HttpClient,
    private val sseClient: SseClient,
    private val httpClientFactory: WebHostHttpClientFactory
) : WebHostApiService {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
    }

    private fun decryptOrParseJsonObject(text: String, token: String): JsonObject {
        if (token.isNotBlank()) {
            val decrypted = runCatching {
                val envelope = json.decodeFromString<EncryptedEnvelope>(text)
                MobileCrypto.decryptJsonObject(envelope, token)
            }.getOrNull()
            if (decrypted != null) return decrypted
        }
        return runCatching { json.parseToJsonElement(text) as? JsonObject }.getOrNull() ?: JsonObject(emptyMap())
    }

    private suspend fun postV2Json(
        hostBaseUrl: String,
        token: String,
        path: String,
        body: JsonObject,
        pinnedCertificateSha256: String,
    ): JsonObject {
        val url = webHostV2ApiUrl(hostBaseUrl, path)
        val useMobileCrypto = token.isNotBlank()
        val bodyText = if (useMobileCrypto) {
            json.encodeToString(MobileCrypto.encryptJsonObject(body, token))
        } else {
            body.toString()
        }
        val response = httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256).post(url) {
            header("X-Nola-Client", "android")
            if (useMobileCrypto) {
                mobileAuthHeaders(token, "POST", url, body = bodyText).forEach { (k, v) -> header(k, v) }
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
            }
            header("Accept", "application/json")
            setBody(TextContent(bodyText, ContentType.Application.Json))
        }
        val text = response.body<String>()
        if (response.status.value !in 200..299) {
            error("WebHost v2 rejected $path (${response.status.value}): $text")
        }
        return decryptOrParseJsonObject(text, token)
    }

    private suspend fun getV2Json(
        hostBaseUrl: String,
        token: String,
        path: String,
        pinnedCertificateSha256: String,
        client: HttpClient = httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        onRoute: ((VerifiedHostTransport) -> Unit)? = null,
    ): JsonObject {
        val url = webHostV2ApiUrl(hostBaseUrl, path)
        val useMobileCrypto = token.isNotBlank()
        val response = client.get(url) {
            header("X-Nola-Client", "android")
            if (useMobileCrypto) {
                mobileAuthHeaders(token, "GET", url).forEach { (k, v) -> header(k, v) }
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
            }
            header("Accept", "application/json")
        }
        val route = if (onRoute == null) null else when {
            response.call.request.attributes.getOrNull(tailcatRoutedRequest) == true -> VerifiedHostTransport.Tailcat
            hostBaseUrl.startsWith("http://", ignoreCase = true) && !httpClientFactory.tailcatAvailable ->
                VerifiedHostTransport.DirectWithoutTailcat
            else -> VerifiedHostTransport.Direct
        }
        val text = response.body<String>()
        if (response.status.value !in 200..299) {
            if (onRoute != null) {
                throw WebHostStatusException(response.status.value, verificationRoute = route)
            }
            error("WebHost v2 GET $path failed (${response.status.value}): $text")
        }
        if (route != null) onRoute?.invoke(route)
        return decryptOrParseJsonObject(text, token)
    }

    override suspend fun v2CreateConversation(
        hostBaseUrl: String,
        token: String,
        title: String,
        pinnedCertificateSha256: String,
    ): String {
        val created = postV2Json(
            hostBaseUrl,
            token,
            "chat/conversations",
            buildJsonObject { put("title", title) },
            pinnedCertificateSha256,
        )
        return (created["conversationId"] as? JsonPrimitive)?.content
            ?.takeIf { it.isNotBlank() }
            ?: error("WebHost v2 returned no conversationId")
    }

    override suspend fun v2DeleteConversation(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ) {
        val url = webHostV2ApiUrl(hostBaseUrl, "chat/conversations/$conversationId")
        val response = httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256).delete(url) {
            header("X-Nola-Client", "android")
            if (token.isNotBlank()) {
                mobileAuthHeaders(token, "DELETE", url).forEach { (k, v) -> header(k, v) }
            }
            header("Accept", "application/json")
        }
        if (response.status.value !in 200..299 && response.status.value != 404) {
            error("WebHost v2 DELETE conversation failed (${response.status.value})")
        }
    }

    override fun v2StreamMessage(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        content: String,
        pinnedCertificateSha256: String,
        attachments: kotlinx.serialization.json.JsonArray?,
        modelId: String?,
        reasoningEffort: String?,
        permissionProfile: String?,
        selectedExperts: kotlinx.serialization.json.JsonArray?,
        deviceBridge: JsonObject?,
    ): Flow<SseEvent> {
        val url = webHostV2ApiUrl(hostBaseUrl, "chat/conversations/$conversationId/messages")
        val payload = buildJsonObject {
            put("content", content)
            if (attachments != null && attachments.isNotEmpty()) put("attachments", attachments)
            if (!modelId.isNullOrBlank()) put("modelId", modelId)
            if (!reasoningEffort.isNullOrBlank()) put("reasoningEffort", reasoningEffort)
            if (!permissionProfile.isNullOrBlank()) put("permissionProfile", permissionProfile)
            if (selectedExperts != null && selectedExperts.isNotEmpty()) put("selected_experts", selectedExperts)
            if (deviceBridge != null) put("device_bridge", deviceBridge)
        }
        val useMobileCrypto = token.isNotBlank()
        val wireBodyJson = if (useMobileCrypto) {
            json.encodeToString(MobileCrypto.encryptJsonObject(payload, token))
        } else {
            payload.toString()
        }
        val headers = buildMap {
            put("X-Nola-Client", "android")
            put("Accept", "text/event-stream")
            if (useMobileCrypto) {
                putAll(mobileAuthHeaders(token, "POST", url, body = wireBodyJson))
                put("X-Nola-Mobile-Crypto", "aes-gcm-v1")
            }
        }
        return sseClient.stream(
            url = url,
            bodyJson = wireBodyJson,
            headers = headers,
            mobileCryptoToken = token.takeIf { useMobileCrypto },
            requestClient = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256),
        )
    }

    override suspend fun v2CancelConversation(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ) {
        postV2Json(
            hostBaseUrl,
            token,
            "chat/conversations/$conversationId/cancel",
            JsonObject(emptyMap()),
            pinnedCertificateSha256,
        )
    }

    override suspend fun v2RespondPermission(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        requestId: String,
        allow: Boolean,
        scope: String,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/permission",
        buildJsonObject {
            put("request_id", requestId)
            put("allow", allow)
            put("scope", scope)
        },
        pinnedCertificateSha256,
    )

    override suspend fun v2RespondUserInput(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        requestId: String,
        answer: String?,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/user-input",
        buildJsonObject {
            put("request_id", requestId)
            if (answer != null) put("answer", answer)
        },
        pinnedCertificateSha256,
    )

    override suspend fun v2ListUiSurfaces(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ): JsonObject = getV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/ui-surfaces",
        pinnedCertificateSha256,
    )

    override suspend fun v2PreviewUiSurface(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        input: JsonObject,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/ui-surface-preview",
        input,
        pinnedCertificateSha256,
    )

    override suspend fun v2SubmitUiInteraction(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        interaction: JsonObject,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/ui-interactions",
        interaction,
        pinnedCertificateSha256,
    )

    override suspend fun v2TruncateConversation(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        fromSeq: Long,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/truncate",
        buildJsonObject { put("fromSeq", fromSeq) },
        pinnedCertificateSha256,
    )

    override suspend fun v2CompactConversation(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/compact",
        JsonObject(emptyMap()),
        pinnedCertificateSha256,
    )

    override suspend fun v2BranchConversation(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ): JsonObject = postV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/branch",
        JsonObject(emptyMap()),
        pinnedCertificateSha256,
    )

    override suspend fun v2WorkspaceReview(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        pinnedCertificateSha256: String,
    ): JsonObject = getV2Json(
        hostBaseUrl,
        token,
        "chat/conversations/$conversationId/workspace-review",
        pinnedCertificateSha256,
    )

    override suspend fun expertMessageCommand(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(operation in setOf("parent", "admit", "manifest"))
        return postV2Json(hostBaseUrl, token, "agent-rooms", request, pinnedCertificateSha256)
    }

    override suspend fun goalControl(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content?.takeIf { it.isNotBlank() }
            ?: (request["conversationId"] as? JsonPrimitive)?.content?.takeIf { it.isNotBlank() }
            ?: ""
        return postV2Json(hostBaseUrl, token, "chat/conversations/$convId/goal", request, pinnedCertificateSha256)
    }

    override suspend fun conversationProgress(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content.orEmpty()
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId/progress", pinnedCertificateSha256)
    }

    override suspend fun submissionFrontier(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content.orEmpty()
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId/configuration", pinnedCertificateSha256)
    }

    override suspend fun permissionConfiguration(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content.orEmpty()
        val url = webHostV2ApiUrl(hostBaseUrl, "chat/conversations/$convId/configuration")
        val op = (request["operation"] as? JsonPrimitive)?.content
        if (op == "patch") {
            val useMobileCrypto = token.isNotBlank()
            val bodyText = if (useMobileCrypto) {
                json.encodeToString(MobileCrypto.encryptJsonObject(request, token))
            } else {
                request.toString()
            }
            val response = httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256).patch(url) {
                header("X-Nola-Client", "android")
                if (useMobileCrypto) {
                    mobileAuthHeaders(token, "PATCH", url, body = bodyText).forEach { (k, v) -> header(k, v) }
                    header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
                }
                header("Accept", "application/json")
                setBody(TextContent(bodyText, ContentType.Application.Json))
            }
            val text = response.body<String>()
            if (response.status.value !in 200..299) error("WebHost v2 PATCH configuration failed (${response.status.value})")
            return decryptOrParseJsonObject(text, token)
        }
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId/configuration", pinnedCertificateSha256)
    }

    override suspend fun auxiliaryCommand(hostBaseUrl: String, token: String, action: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(action in setOf("prepare", "admit", "invoke", "inspect", "cancel"))
        return postV2Json(hostBaseUrl, token, "auxiliary/$action", request, pinnedCertificateSha256)
    }

    override suspend fun agentRoomCommand(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject =
        postV2Json(hostBaseUrl, token, "agent-rooms", request, pinnedCertificateSha256)

    override suspend fun generativeUi(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(operation in GENERATIVE_UI_OPERATIONS)
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content
            ?: ((request["identity"] as? JsonObject)?.get("owner") as? JsonObject)
                ?.get("conversation_id")?.let { (it as? JsonPrimitive)?.content }
            ?: ""
        return when (operation) {
            "surfaces" -> getV2Json(hostBaseUrl, token, "chat/conversations/$convId/ui-surfaces", pinnedCertificateSha256)
            "preview" -> postV2Json(hostBaseUrl, token, "chat/conversations/$convId/ui-surface-preview", request, pinnedCertificateSha256)
            "changes" -> postV2Json(hostBaseUrl, token, "chat/conversations/$convId/ui-surface-changes", request, pinnedCertificateSha256)
            "revision" -> postV2Json(hostBaseUrl, token, "chat/conversations/$convId/ui-surface-revision", request, pinnedCertificateSha256)
            "interactions" -> postV2Json(hostBaseUrl, token, "chat/conversations/$convId/ui-interactions", request, pinnedCertificateSha256)
            "queued-input" -> getV2Json(hostBaseUrl, token, "chat/conversations/$convId/events", pinnedCertificateSha256)
            "queued-input/cancel" -> postV2Json(hostBaseUrl, token, "chat/conversations/$convId/cancel", request, pinnedCertificateSha256)
            else -> error("Unsupported generative UI operation: $operation")
        }
    }

    override suspend fun readAssistantJournal(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content.orEmpty()
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId/messages", pinnedCertificateSha256)
    }

    override suspend fun observeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content.orEmpty()
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId/observe", pinnedCertificateSha256)
    }

    override fun resumeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): Flow<SseEvent> {
        val convId = (request["conversation_id"] as? JsonPrimitive)?.content
            ?: (request["conversationId"] as? JsonPrimitive)?.content
            ?: ""
        val prompt = (request["content"] as? JsonPrimitive)?.content.orEmpty()
        return v2StreamMessage(hostBaseUrl, token, convId, prompt, pinnedCertificateSha256)
    }

    override suspend fun requestWebManagementUrl(
        hostBaseUrl: String,
        token: String,
        target: String,
        pinnedCertificateSha256: String
    ): String {
        val handoffTarget = runCatching {
            val response = postV2Json(
                hostBaseUrl = hostBaseUrl,
                token = token,
                path = "web-handoff",
                body = buildJsonObject { put("target", target) },
                pinnedCertificateSha256 = pinnedCertificateSha256,
            )
            (response["url"] as? JsonPrimitive)?.content?.takeIf { it.isNotBlank() }
                ?: (response["path"] as? JsonPrimitive)?.content?.takeIf { it.isNotBlank() }
        }.getOrNull()
        return webHostControlPlaneUrl(hostBaseUrl, handoffTarget ?: target)
            ?: webHostControlPlaneUrl(hostBaseUrl, target)
            ?: error("Web Agent Host returned an invalid management URL")
    }

    override suspend fun getAgentSettings(
        hostBaseUrl: String,
        token: String,
        pinnedCertificateSha256: String
    ): AgentSettingsResponse {
        var verifiedTransport: VerifiedHostTransport? = null
        val body = getV2Json(
            hostBaseUrl = hostBaseUrl,
            token = token,
            path = "agent/settings",
            pinnedCertificateSha256 = pinnedCertificateSha256,
            client = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256),
            onRoute = { verifiedTransport = it },
        )
        return json.decodeFromJsonElement<AgentSettingsResponse>(body).copy(verifiedTransport = verifiedTransport)
    }

    override suspend fun submitAgentPermissionDecision(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String
    ) {
        val convId = payload["session_id"]?.toString()
            ?: payload["conversation_id"]?.toString()
            ?: "default"
        val reqId = payload["request_id"]?.toString()
            ?: payload["id"]?.toString()
            ?: ""
        val resolutionDecision = (payload["resolution"] as? Map<*, *>)?.get("decision")?.toString()?.lowercase()
        val decisionStr = payload["decision"]?.toString()?.lowercase() ?: resolutionDecision
        val allow = (payload["allow"] as? Boolean)
            ?: (payload["approved"] as? Boolean)
            ?: (decisionStr == "allow" || decisionStr == "approve" || decisionStr == "allow_once" || decisionStr == "allow_conversation")
        val scope = payload["scope"]?.toString()?.takeIf { it.isNotBlank() }
            ?: if (decisionStr == "allow_conversation" || payload["remember_for_conversation"] == true) "session" else "once"
        v2RespondPermission(
            hostBaseUrl = hostBaseUrl,
            token = token,
            conversationId = convId,
            requestId = reqId,
            allow = allow,
            scope = scope,
            pinnedCertificateSha256 = pinnedCertificateSha256,
        )
    }

    override suspend fun cancelAdmittedRemoteSubmission(
        hostBaseUrl: String,
        token: String,
        identity: RemoteSubmissionIdentity,
        pinnedCertificateSha256: String
    ) {
        v2CancelConversation(hostBaseUrl, token, identity.conversationId, pinnedCertificateSha256)
    }

    override suspend fun submitAgentDeviceToolResult(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String
    ) {
        val convId = payload["session_id"]?.toString()?.takeIf { it.isNotBlank() }
            ?: payload["conversation_id"]?.toString().orEmpty()
        postV2Json(
            hostBaseUrl,
            token,
            "chat/conversations/$convId/device-tool",
            MobileCrypto.payloadJsonObject(payload),
            pinnedCertificateSha256,
        )
    }

    override suspend fun taskCancellationCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversationId"] as? JsonPrimitive)?.content.orEmpty()
        return postV2Json(hostBaseUrl, token, "chat/conversations/$convId/cancel", request, pinnedCertificateSha256)
    }

    override suspend fun taskGuidanceCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversationId"] as? JsonPrimitive)?.content.orEmpty()
        return postV2Json(hostBaseUrl, token, "chat/conversations/$convId/messages", request, pinnedCertificateSha256)
    }

    override fun streamAgentTaskEvents(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        afterCursor: Long,
        pinnedCertificateSha256: String,
    ): Flow<SseEvent> {
        val query = if (afterCursor > 0L) "?afterCursor=$afterCursor" else ""
        val url = webHostV2ApiUrl(hostBaseUrl, "chat/conversations/$conversationId/tasks/events$query")
        val useMobileCrypto = token.isNotBlank()
        val headers = buildMap {
            put("X-Nola-Client", "android")
            put("Accept", "text/event-stream")
            if (useMobileCrypto) {
                putAll(mobileAuthHeaders(token, "GET", url))
                put("X-Nola-Mobile-Crypto", "aes-gcm-v1")
            }
        }
        return sseClient.streamGet(
            url = url,
            headers = headers,
            mobileCryptoToken = token.takeIf { useMobileCrypto },
            requestClient = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256),
        )
    }

    override suspend fun remoteSubmission(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val convId = (request["conversationId"] as? JsonPrimitive)?.content
            ?: ((request["identity"] as? JsonObject)?.get("conversation_id") as? JsonPrimitive)?.content
            ?: ""
        return getV2Json(hostBaseUrl, token, "chat/conversations/$convId", pinnedCertificateSha256)
    }

    override suspend fun prepareToolPlan(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject =
        postV2Json(hostBaseUrl, token, "agent/tool-plan/prepare", request, pinnedCertificateSha256)

    override fun streamBoundAgentRequest(
        hostBaseUrl: String,
        token: String,
        submission: FrozenRemoteSubmission,
        pinnedCertificateSha256: String,
    ): Flow<SseEvent> {
        val lastMessage = (submission.request["messages"] as? kotlinx.serialization.json.JsonArray)
            ?.lastOrNull() as? JsonObject
        val prompt = lastMessage
            ?.get("content")
            ?.let { (it as? JsonPrimitive)?.content }
            .orEmpty()
        val attachments = (submission.request["attachments"] as? kotlinx.serialization.json.JsonArray)
            ?: (lastMessage?.get("attachments") as? kotlinx.serialization.json.JsonArray)
        val modelId = (submission.request["modelId"] as? JsonPrimitive)?.content
            ?: (submission.request["model_id"] as? JsonPrimitive)?.content
            ?: ((submission.request["model_intent"] as? JsonObject)?.get("model_id") as? JsonPrimitive)?.content
        val reasoningEffort = (submission.request["reasoningEffort"] as? JsonPrimitive)?.content
            ?: (submission.request["reasoning_effort"] as? JsonPrimitive)?.content
        val permissionProfile = (submission.request["permissionProfile"] as? JsonPrimitive)?.content
            ?: (submission.request["permission_profile"] as? JsonPrimitive)?.content
        val selectedExperts = submission.request["selected_experts"] as? kotlinx.serialization.json.JsonArray
        val deviceBridge = submission.request["device_bridge"] as? JsonObject
        return v2StreamMessage(
            hostBaseUrl = hostBaseUrl,
            token = token,
            conversationId = submission.identity.conversationId,
            content = prompt,
            pinnedCertificateSha256 = pinnedCertificateSha256,
            attachments = attachments,
            modelId = modelId,
            reasoningEffort = reasoningEffort,
            permissionProfile = permissionProfile,
            selectedExperts = selectedExperts,
            deviceBridge = deviceBridge,
        )
    }
}

class WebHostStatusException(val statusCode: Int, val errorCode: String? = null, val expertRejection: JsonObject? = null,
    val verificationRoute: VerifiedHostTransport? = null) :
    IllegalStateException("Web Host request failed ($statusCode)" + (errorCode?.let { ": $it" } ?: ""))

internal fun boundedWebHostErrorCode(error: JsonObject): String? =
    (error["code"] as? kotlinx.serialization.json.JsonPrimitive)?.takeIf { it.isString }?.content
        ?.takeIf { it.matches(Regex("[a-z][a-z0-9_]{0,95}")) }

/** Only bounded source identities are retained; arbitrary Host error text is never journaled. */
internal fun boundedExpertRejection(value: JsonObject): JsonObject? {
    if (value["code"] != JsonPrimitive("expert_admission_rejected") || value["definitive_rejection"] != JsonPrimitive(true)) return null
    val fields = listOf("host_id", "client_id", "conversation_id", "submission_id", "user_message_id")
    if (fields.any { key -> (value[key] as? JsonPrimitive)?.let { !it.isString || it.content.isBlank() || it.content.length > 512 || it.content.any(Char::isISOControl) } != false }) return null
    return buildJsonObject { put("code", "expert_admission_rejected"); put("definitive_rejection", true); fields.forEach { put(it, value.getValue(it)) } }
}

/** Shared production control transport; a 4xx is never a successful absence. */
internal suspend fun postEncryptedWebHostRequest(client: HttpClient, url: String, token: String,
    request: JsonObject, maximumRequestBytes: Int = 64 * 1024,
    maximumResponseBytes: Int = 16 * 1024 * 1024): JsonObject {
    require(maximumResponseBytes in 1..16 * 1024 * 1024)
    require(request.toString().toByteArray(Charsets.UTF_8).size <= maximumRequestBytes)
    val body = Json { encodeDefaults = true }.encodeToString(MobileCrypto.encryptJsonObject(request, token))
    return client.preparePost(url) {
        mobileAuthHeaders(token, "POST", url, body = body).forEach { (name, value) -> header(name, value) }
        header("Accept", "application/json")
        header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
        setBody(TextContent(body, ContentType.Application.Json))
    }.execute { response ->
        if (response.status.value !in 200..299) {
            // Only an authenticated, bounded protocol code is exposed; never log arbitrary Host text.
            var expertRejection: JsonObject? = null
            val code = try {
                val errorBytes = response.bodyAsChannel().readRemaining(16_385).readBytes()
                require(errorBytes.size <= 16_384)
                val errorEnvelope = Json.decodeFromString<EncryptedEnvelope>(errorBytes.decodeToString(throwOnInvalidSequence = true))
                val error = MobileCrypto.decryptJsonObject(errorEnvelope, token)
                expertRejection = boundedExpertRejection(error)
                boundedWebHostErrorCode(error)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) { null }
            throw WebHostStatusException(response.status.value, code, expertRejection)
        }
        // Read at most one byte beyond the wire budget, including chunked responses.
        // Streaming execute also closes the response when validation throws.
        val bytes = response.bodyAsChannel().readRemaining(maximumResponseBytes.toLong() + 1).readBytes()
        require(bytes.size <= maximumResponseBytes) { "Web Host response exceeds byte limit" }
        val envelope = Json.decodeFromString<EncryptedEnvelope>(bytes.decodeToString(throwOnInvalidSequence = true))
        MobileCrypto.decryptJsonObject(envelope, token)
    }
}

internal fun mobileAuthHeaders(
    token: String,
    method: String,
    url: String,
    timestamp: Long = System.currentTimeMillis() / 1_000L,
    nonce: String = UUID.randomUUID().toString().lowercase(),
    body: String = ""
): Map<String, String> {
    val bodyDigest = MessageDigest.getInstance("SHA-256")
        .digest(body.toByteArray(Charsets.UTF_8))
        .joinToString("") { "%02x".format(it.toInt() and 0xff) }
    val proof = mobileAuthProof(token, method, url, timestamp, nonce, bodyDigest)
    return mapOf(
        "X-Nola-Mobile-Timestamp" to timestamp.toString(),
        "X-Nola-Mobile-Nonce" to nonce,
        "X-Nola-Mobile-Body-SHA256" to bodyDigest,
        "X-Nola-Mobile-Auth" to proof
    )
}

/** Match a frozen credential without consuming/re-serializing the HTTP body. */
internal fun mobileAuthProof(token: String, method: String, url: String,
    timestamp: Long, nonce: String, bodyDigest: String): String {
    val path = runCatching { URI(url).rawPath + (URI(url).rawQuery?.let { "?$it" } ?: "") }.getOrDefault("/")
    val message = "v1\n${method.trim().uppercase()}\n$path\n$timestamp\n${nonce.trim()}\n$bodyDigest"
    val mac = Mac.getInstance("HmacSHA256")
    mac.init(SecretKeySpec(token.toByteArray(Charsets.UTF_8), "HmacSHA256"))
    return Base64.getUrlEncoder().withoutPadding().encodeToString(mac.doFinal(message.toByteArray(Charsets.UTF_8)))
}

internal fun normalizeWebHostBaseUrl(raw: String): String {
    var value = raw.trim()
    if (value.isBlank()) return ""
    if (!value.startsWith("http://", ignoreCase = true) && !value.startsWith("https://", ignoreCase = true)) {
        value = "http://$value"
    }
    value = value.substringBefore('#').substringBefore('?').trimEnd('/')
    return value
        .replace(Regex("/api/mobile(?:/.*)?$", RegexOption.IGNORE_CASE), "")
        .replace(Regex("/api(?:/.*)?$", RegexOption.IGNORE_CASE), "")
        .replace(Regex("/v\\d+(?:/.*)?$", RegexOption.IGNORE_CASE), "")
        .trimEnd('/')
}

internal fun isAllowedWebHostTransport(raw: String): Boolean {
    val uri = runCatching { URI(normalizeWebHostBaseUrl(raw)) }.getOrNull() ?: return false
    if (uri.userInfo != null) return false
    val scheme = uri.scheme?.lowercase() ?: return false
    if (scheme == "https") return !uri.host.isNullOrBlank()
    if (scheme != "http") return false
    val host = uri.host?.trim()?.trim('[', ']')?.lowercase().orEmpty()
    if (host.isEmpty()) return false
    if (host == "localhost" || host == "localhost.localdomain" || host.endsWith(".localhost") || host.endsWith(".local")) {
        return true
    }
    if (host == "::1" || host == "0:0:0:0:0:0:0:1") return true
    if (host.contains(':')) {
        val firstGroup = host.substringBefore(':').toIntOrNull(16) ?: return false
        return firstGroup in 0xfc00..0xfdff || firstGroup in 0xfe80..0xfebf
    }
    val labels = host.split('.')
    if (labels.size != 4) return false
    val parts = labels.mapNotNull(String::toIntOrNull)
    if (parts.size != 4 || parts.any { it !in 0..255 }) return false
    return parts[0] == 10 ||
        parts[0] == 127 ||
        (parts[0] == 172 && parts[1] in 16..31) ||
        (parts[0] == 192 && parts[1] == 168) ||
        (parts[0] == 169 && parts[1] == 254)
}

internal fun webHostV2ApiUrl(hostBaseUrl: String, path: String): String {
    val base = normalizeWebHostBaseUrl(hostBaseUrl)
    return "$base/api/mobile/${path.trimStart('/')}"
}

internal fun webHostMobileUrl(hostBaseUrl: String, path: String): String =
    webHostV2ApiUrl(hostBaseUrl, path)

internal fun webHostControlPlaneUrl(hostBaseUrl: String, path: String): String? {
    val base = runCatching { URI(normalizeWebHostBaseUrl(hostBaseUrl)) }.getOrNull() ?: return null
    val scheme = base.scheme ?: return null
    val host = base.host ?: return null
    val requested = runCatching { URI(path.trim()) }.getOrNull()
    if (requested?.isAbsolute == true) {
        return requested.takeIf { it.scheme == "http" || it.scheme == "https" }?.toString()
    }
    val cleanPath = requested?.path.orEmpty().trimStart('/').ifEmpty { "settings" }
    return runCatching {
        URI(scheme, null, host, base.port, "/$cleanPath", requested?.query, requested?.fragment).toString()
    }.getOrNull()
}

internal fun rewriteLoopbackHttpUrlForWebHost(rawUrl: String, hostBaseUrl: String): String {
    val rawUri = runCatching { URI(rawUrl) }.getOrNull() ?: return rawUrl
    val scheme = rawUri.scheme?.lowercase()
    if (scheme != "http" && scheme != "https") return rawUrl
    if (!rawUri.host.isAndroidLoopbackHost()) return rawUrl

    val baseUri = runCatching { URI(normalizeWebHostBaseUrl(hostBaseUrl)) }.getOrNull()
    val replacementHost = baseUri?.host?.takeIf { it.isNotBlank() } ?: return rawUrl
    val replacementPort = rawUri.port.takeIf { it >= 0 } ?: (baseUri?.port ?: -1)
    return runCatching {
        URI(
            scheme,
            null,
            replacementHost,
            replacementPort,
            rawUri.path,
            rawUri.query,
            rawUri.fragment
        ).toString()
    }.getOrDefault(rawUrl)
}

private fun String?.isAndroidLoopbackHost(): Boolean {
    val host = this?.trim()?.trim('[', ']')?.trimEnd('.')?.lowercase().orEmpty()
    if (host == "localhost" || host == "localhost.localdomain" || host == "::1" || host == "0:0:0:0:0:0:0:1") {
        return true
    }
    val parts = host.split('.')
    return parts.size == 4 &&
        parts.first() == "127" &&
        parts.all { part -> part.toIntOrNull()?.let { it in 0..255 } == true }
}

internal fun buildWebHostAgentRequestBody(
    messages: List<JsonObject>,
    conversationId: String?,
    allowLocalDevTools: Boolean,
    maxIterations: Int,
    timezone: String,
    language: String,
    agentMode: Boolean,
    permissionProfile: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
    responseStyle: String,
    customSystemPrompt: String,
    nativeAppHostId: String,
    expectedHostId: String = "",
    clientId: String = "",
    deviceBridge: JsonObject? = null,
    executionBackend: String = "nola",
    externalSessionId: String = "",
    submissionId: String? = null,
    queuedUserCommandId: String? = null,
): Map<String, Any?> = buildMap {
    submissionId?.let {
        require(it.isNotBlank() && !conversationId.isNullOrBlank())
        put("submission_id", it); put("user_message_id", it); put("assistant_message_id", "$it:assistant")
    }
    queuedUserCommandId?.let {
        require(it == submissionId && it.isNotEmpty() && it.toByteArray(Charsets.UTF_8).size <= 128 &&
            it.trim() == it && it.none(Char::isISOControl)) { "Invalid queued UI command identity" }
        require(executionBackend == "nola" && externalSessionId.isEmpty())
        put("queued_user_command_id", it)
        put("user_message_id", "ui:$it:user")
    }
    put("messages", messages)
    put("allow_local_dev_tools", allowLocalDevTools)
    put("max_iterations", maxIterations)
    put("timezone", timezone)
    if (language.isNotBlank()) {
        put("language", language)
    }
    put("agent_mode", agentMode)
    put("permission_profile", permissionProfile.wireValue)
    if (responseStyle.isNotBlank()) {
        put("response_style", responseStyle)
    }
    if (customSystemPrompt.isNotBlank()) {
        put("custom_system_prompt", customSystemPrompt)
    }
    if (nativeAppHostId.isNotBlank()) {
        put("native_app_host_id", nativeAppHostId)
    }
    if (!conversationId.isNullOrEmpty()) {
        put("conversationId", conversationId)
    }
    if (expectedHostId.isNotBlank()) {
        put("expected_host_id", expectedHostId)
    }
    if (clientId.isNotBlank()) {
        put("client_id", clientId)
    }
    if (deviceBridge != null) {
        put("device_bridge", deviceBridge)
    }
    put("execution_backend", executionBackend.ifBlank { "nola" })
    if (externalSessionId.isNotBlank()) {
        put("external_session_id", externalSessionId)
    }
}
