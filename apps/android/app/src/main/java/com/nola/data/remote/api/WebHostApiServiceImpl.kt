package com.nola.data.remote.api

import com.nola.core.AndroidAgentPermissionMode

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.decodeAgentCoreTaskProjection
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.get
import io.ktor.client.request.header
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

    override suspend fun expertMessageCommand(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(operation in setOf("parent", "admit", "manifest"))
        return postEncryptedWebHostRequest(httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
            webHostMobileUrl(hostBaseUrl, "chat/expert-messages/$operation"), token, request,
            maximumRequestBytes = 4 * 1024 * 1024, maximumResponseBytes = 8 * 1024 * 1024)
    }

    override suspend fun goalControl(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/goal"), token, request,
    )

    override suspend fun conversationProgress(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/progress"), token, request,
    )

    override suspend fun submissionFrontier(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/submission-frontier"), token, request,
    )

    override suspend fun permissionConfiguration(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/configuration"), token, request,
    )

    override suspend fun auxiliaryCommand(hostBaseUrl: String, token: String, action: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(action in setOf("prepare", "admit", "invoke", "inspect", "cancel"))
        return postEncryptedWebHostRequest(httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
            webHostMobileUrl(hostBaseUrl, "auxiliary/$action"), token, request)
    }

    override suspend fun agentRoomCommand(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "agent-rooms"), token, request,
    )

    override suspend fun generativeUi(hostBaseUrl: String, token: String, operation: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        require(operation in setOf("surfaces", "interactions", "queued-input", "queued-input/cancel", "revision", "changes"))
        return postEncryptedWebHostRequest(httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
            webHostMobileUrl(hostBaseUrl, "chat/agent/v2/ui/$operation"), token, request, 128 * 1024,
            maximumResponseBytes = when (operation) {
                "surfaces" -> 16 * 1024 * 1024
                "interactions", "revision", "changes" -> 2 * 1024 * 1024
                else -> 64 * 1024
            })
    }

    override suspend fun readAssistantJournal(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/journal")
        return postEncryptedWebHostRequest(httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
            url, token, request)
    }

    override suspend fun observeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/observe"), token, request,
    )

    override fun resumeRemoteConversation(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): Flow<SseEvent> {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/resume")
        val body = json.encodeToString(MobileCrypto.encryptJsonObject(request, token))
        return sseClient.stream(url = url, bodyJson = body,
            headers = mobileAuthHeaders(token, "POST", url, body = body) + mapOf(
                "Accept" to "text/event-stream", "X-Nola-Mobile-Crypto" to "aes-gcm-v1"),
            mobileCryptoToken = token,
            requestClient = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256))
    }

    override suspend fun requestWebManagementUrl(
        hostBaseUrl: String,
        token: String,
        target: String,
        pinnedCertificateSha256: String
    ): String {
        val url = webHostMobileUrl(hostBaseUrl, "web-handoff")
        val encryptedBodyJson = json.encodeToString(MobileCrypto.encryptJson(mapOf("target" to target), token))
        val response = httpClientFactory
            .clientFor(hostBaseUrl, pinnedCertificateSha256)
            .post(url) {
                mobileAuthHeaders(token, "POST", url, body = encryptedBodyJson)
                    .forEach { (name, value) -> header(name, value) }
                header("Accept", "application/json")
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
                setBody(TextContent(encryptedBodyJson, ContentType.Application.Json))
            }
        if (response.status.value !in 200..299) {
            error("Web Agent Host rejected the browser handoff (${response.status.value})")
        }
        val envelope: EncryptedEnvelope = response.body()
        val decrypted = MobileCrypto.decryptJsonObject(envelope, token)
        val path = decrypted["path"]?.toString()?.trim('"')
            ?.takeIf { it.isNotBlank() }
            ?: error("Web Agent Host returned an invalid browser handoff")
        return webHostControlPlaneUrl(hostBaseUrl, path)
            ?: error("Web Agent Host returned an invalid browser handoff URL")
    }

    override suspend fun getAgentSettings(
        hostBaseUrl: String,
        token: String,
        pinnedCertificateSha256: String
    ): AgentSettingsResponse {
        val envelope = httpGetEncrypted(
            client = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256),
            hostBaseUrl = hostBaseUrl,
            token = token,
            path = "agent/settings"
        )
        val decrypted = MobileCrypto.decryptJsonObject(envelope, token)
        return json.decodeFromJsonElement(decrypted)
    }

    override suspend fun submitAgentPermissionDecision(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String
    ) {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/permission")
        val encryptedBodyJson = json.encodeToString(MobileCrypto.encryptJson(payload, token))
        val response = httpClientFactory
            .controlClientFor(hostBaseUrl, pinnedCertificateSha256)
            .post(url) {
                mobileAuthHeaders(token, "POST", url, body = encryptedBodyJson)
                    .forEach { (name, value) -> header(name, value) }
                header("Accept", "application/json")
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
                setBody(TextContent(encryptedBodyJson, ContentType.Application.Json))
            }
        if (response.status.value !in 200..299) {
            error("Web Agent Host rejected the permission response (${response.status.value})")
        }
    }

    override suspend fun cancelAdmittedRemoteSubmission(
        hostBaseUrl: String,
        token: String,
        identity: RemoteSubmissionIdentity,
        requestDigest: String,
        pinnedCertificateSha256: String
    ) {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/cancel")
        val payload = kotlinx.serialization.json.buildJsonObject {
            put("identity", identity.wire()); put("request_digest", kotlinx.serialization.json.JsonPrimitive(requestDigest))
        }
        val encryptedBodyJson = json.encodeToString(MobileCrypto.encryptJsonObject(payload, token))
        val response = httpClientFactory
            .controlClientFor(hostBaseUrl, pinnedCertificateSha256)
            .post(url) {
                mobileAuthHeaders(token, "POST", url, body = encryptedBodyJson)
                    .forEach { (name, value) -> header(name, value) }
                header("Accept", "application/json")
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
                setBody(TextContent(encryptedBodyJson, ContentType.Application.Json))
            }
        if (response.status.value !in 200..299) {
            error("Web Agent Host rejected cancellation (${response.status.value})")
        }
        val envelope: EncryptedEnvelope = response.body()
        val result = MobileCrypto.decryptJsonObject(envelope, token)
        val cancelRequested = (result["cancelRequested"] as? kotlinx.serialization.json.JsonPrimitive)
            ?.content
            ?.toBooleanStrictOrNull()
            ?: false
        check(cancelRequested) {
            "Web Agent Host did not find an active generation for this conversation"
        }
    }

    override suspend fun submitAgentDeviceToolResult(
        hostBaseUrl: String,
        token: String,
        payload: Map<String, Any>,
        pinnedCertificateSha256: String
    ) {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/device-tool")
        val encryptedBodyJson = json.encodeToString(MobileCrypto.encryptJson(payload, token))
        val response = httpClientFactory
            .clientFor(hostBaseUrl, pinnedCertificateSha256)
            .post(url) {
                mobileAuthHeaders(token, "POST", url, body = encryptedBodyJson)
                    .forEach { (name, value) -> header(name, value) }
                header("Accept", "application/json")
                header("X-Nola-Mobile-Crypto", "aes-gcm-v1")
                setBody(TextContent(encryptedBodyJson, ContentType.Application.Json))
            }
        if (response.status.value !in 200..299) {
            error("Web Agent Host rejected the device tool result (${response.status.value})")
        }
    }

    override suspend fun taskCancellationCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/task/cancel" + if (receiptOnly) "/receipt" else ""), token, request,
    )

    override suspend fun taskGuidanceCommand(hostBaseUrl: String, token: String, request: JsonObject,
        receiptOnly: Boolean, pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/task/message" + if (receiptOnly) "/receipt" else ""), token, request,
    )

    override fun streamAgentTaskEvents(
        hostBaseUrl: String,
        token: String,
        conversationId: String,
        afterCursor: Long,
        pinnedCertificateSha256: String,
    ): Flow<SseEvent> {
        val url = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/task/events")
        val body = mapOf(
            "conversationId" to conversationId,
            "afterCursor" to afterCursor.coerceAtLeast(0),
            "timeoutMs" to 25_000,
        )
        val encryptedBodyJson = json.encodeToString(MobileCrypto.encryptJson(body, token))
        val headers = mobileAuthHeaders(token, "POST", url, body = encryptedBodyJson) + mapOf(
            "Accept" to "text/event-stream",
            "X-Nola-Mobile-Crypto" to "aes-gcm-v1",
        )
        return sseClient.stream(
            url = url,
            bodyJson = encryptedBodyJson,
            headers = headers,
            mobileCryptoToken = token,
            requestClient = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256),
        )
    }

    override suspend fun remoteSubmission(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "chat/agent/v2/submission"), token, request, 17 * 1024 * 1024)

    override suspend fun prepareToolPlan(hostBaseUrl: String, token: String, request: JsonObject,
        pinnedCertificateSha256: String): JsonObject = postEncryptedWebHostRequest(
        httpClientFactory.controlClientFor(hostBaseUrl, pinnedCertificateSha256),
        webHostMobileUrl(hostBaseUrl, "agent/tool-plan/prepare"), token, request, 512 * 1024)

    override fun streamBoundAgentRequest(
        hostBaseUrl: String,
        token: String,
        submission: FrozenRemoteSubmission,
        pinnedCertificateSha256: String,
    ): Flow<SseEvent> {
        val encryptedBody = MobileCrypto.encryptJsonObject(submission.wireRequest(), token)
        val encryptedBodyJson = json.encodeToString(encryptedBody)

        val streamUrl = webHostMobileUrl(hostBaseUrl, "chat/agent/v2/stream")
        val headers = mobileAuthHeaders(token, "POST", streamUrl, body = encryptedBodyJson) + mapOf(
            "Accept" to "text/event-stream",
            "X-Nola-Mobile-Crypto" to "aes-gcm-v1"
        )

        return sseClient.stream(
            url = streamUrl,
            bodyJson = encryptedBodyJson,
            headers = headers,
            mobileCryptoToken = token,
            requestClient = httpClientFactory.clientFor(hostBaseUrl, pinnedCertificateSha256)
        )
    }

    private suspend fun httpGetEncrypted(
        client: HttpClient = httpClient,
        hostBaseUrl: String,
        token: String,
        path: String
    ): EncryptedEnvelope {
        val url = webHostMobileUrl(hostBaseUrl, path)
        return client.get(url) {
            mobileAuthHeaders(token, "GET", url).forEach { (name, value) -> header(name, value) }
            header("Accept", "application/json")
        }.body()
    }
}

class WebHostStatusException(val statusCode: Int, val errorCode: String? = null, val expertRejection: JsonObject? = null) :
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

internal fun webHostMobileUrl(hostBaseUrl: String, path: String): String {
    val base = normalizeWebHostBaseUrl(hostBaseUrl)
    return "$base/api/mobile/${path.trimStart('/')}"
}

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
