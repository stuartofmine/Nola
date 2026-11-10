package com.nola.data.remote.sse

import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.agent.trace.AgentTraceEvent
import io.ktor.client.HttpClient
import io.ktor.client.request.preparePost
import io.ktor.client.request.setBody
import com.nola.data.remote.applyStreamingTimeouts
import io.ktor.client.statement.bodyAsChannel
import io.ktor.http.ContentType
import io.ktor.http.content.TextContent
import io.ktor.http.contentType
import io.ktor.utils.io.readUTF8Line
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.nola.app.sse.SseEventDto
import javax.inject.Inject
import javax.inject.Singleton

sealed class SseEvent {
    data class SubmissionReceipt(val receipt: kotlinx.serialization.json.JsonObject) : SseEvent()
    // Hints only: task subscriptions, permission challenges and journals remain authoritative.
    data class CoreProjectionHint(val conversationId: String, val type: String) : SseEvent()
    data class ContextUsageChanged(val envelope: kotlinx.serialization.json.JsonObject) : SseEvent()
    data class AssistantJournal(val envelope: kotlinx.serialization.json.JsonObject) : SseEvent()
    data class AssistantTerminal(val envelope: kotlinx.serialization.json.JsonObject) : SseEvent()
    data class Trace(val event: AgentTraceEvent) : SseEvent()
    data class TurnLifecycle(val envelope: kotlinx.serialization.json.JsonObject) : SseEvent()
    data class TaskProjection(
        val conversationId: String,
        val cursor: Long,
        val projection: com.nola.core.AgentCoreTaskProjection?,
    ) : SseEvent()
    data class TaskStreamCheckpoint(
        val conversationId: String,
        val cursor: Long,
    ) : SseEvent()
    data class PermissionRequest(
        val allowConversation: Boolean,
        val id: String,
        val toolName: String,
        val args: Map<String, String>,
        val match: String,
        val reason: String,
        val permissionInstructions: List<com.nola.core.AgentPermissionInstruction> = emptyList(),
        val message: String,
        val nonce: String? = null,
        val hostId: String? = null,
        val clientId: String? = null,
        val sessionId: String? = null,
        val expiresAt: Long? = null,
        val protocolVersion: Int? = null,
        val safeReviewText: String? = null
    ) : SseEvent()
    data class DeviceToolRequest(
        val id: String,
        val nonce: String,
        val hostId: String,
        val clientId: String,
        val sessionId: String,
        val toolName: String,
        val capability: String,
        val arguments: kotlinx.serialization.json.JsonObject,
        val args: Map<String, String>,
        val expiresAt: Long,
        val protocolVersion: Int
    ) : SseEvent()
    data class ExternalSessionStarted(val backendId: String, val sessionId: String) : SseEvent()
    data class Error(val message: String) : SseEvent()
}

class SseStatusException(val statusCode: Int) : IllegalStateException("SSE request failed ($statusCode)")

@Singleton
class SseClient @Inject constructor(
    private val httpClient: HttpClient
) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    fun stream(
        url: String,
        bodyJson: String,
        headers: Map<String, String>,
        mobileCryptoToken: String? = null,
        requestClient: HttpClient = httpClient
    ): Flow<SseEvent> = flow {
        requestClient.preparePost(url) {
            headers.forEach { (k, v) -> this.headers.append(k, v) }
            setBody(TextContent(bodyJson, ContentType.Application.Json))
            applyStreamingTimeouts(requestClient)
        }.execute { response ->
            if (response.status.value !in 200..299) {
                throw SseStatusException(response.status.value)
            }
            val channel = response.bodyAsChannel()
            val dataLines = mutableListOf<String>()
            suspend fun flushFrame(): Boolean {
                val payload = sseFramePayload(dataLines) ?: return false
                dataLines.clear()
                if (payload == "[DONE]") return true
                try {
                    decodeSsePayload(payload, mobileCryptoToken, json)?.let { emit(it) }
                } catch (e: Exception) {
                    if (e is IllegalStateException) throw e
                    throw IllegalStateException("Failed to decode Agent Host SSE event", e)
                }
                return false
            }
            while (!channel.isClosedForRead) {
                val line = channel.readUTF8Line() ?: break
                if (line.isEmpty()) {
                    if (flushFrame()) break
                    continue
                }
                sseDataLineValue(line)?.let { dataLines.add(it) }
            }
            if (dataLines.isNotEmpty()) {
                flushFrame()
            }
        }
    }.flowOn(Dispatchers.IO)
}

internal fun sseDataLineValue(line: String): String? {
    if (!line.startsWith("data:")) return null
    val value = line.substringAfter("data:")
    return if (value.startsWith(" ")) value.drop(1) else value
}

internal fun sseFramePayload(dataLines: List<String>): String? {
    if (dataLines.isEmpty()) return null
    return dataLines.joinToString(separator = "\n").trim().ifEmpty { null }
}

internal fun decodeSsePayload(
    payload: String,
    mobileCryptoToken: String?,
    json: Json = Json { ignoreUnknownKeys = true; isLenient = true }
): SseEvent? {
    if (payload == "[DONE]") return null
    val rawObject = json.parseToJsonElement(payload).jsonObject
    fun validateControlTrace(value: kotlinx.serialization.json.JsonObject) {
        val trace = value["trace"] as? kotlinx.serialization.json.JsonObject ?: return
        val fields = trace["payload"] as? kotlinx.serialization.json.JsonObject
        require(fields?.containsKey("delta") != true) { "Trace cannot carry assistant or tool body deltas" }
    }
    val isEncrypted = rawObject["type"]?.jsonPrimitive?.contentOrNull == "encrypted"
    if (!isEncrypted) {
        return try {
            validateControlTrace(rawObject)
            decodeCoreProjectionHint(rawObject) ?: json.decodeFromJsonElement<SseEventDto>(rawObject).toDomain(rawObject)
        } catch (e: Exception) {
            throw IllegalStateException("Failed to decode Agent Host SSE event", e)
        }
    }

    val token = mobileCryptoToken
        ?: throw IllegalStateException("Encrypted SSE event requires a mobile bridge token")
    val eventObject = try {
        val envelope = json.decodeFromJsonElement<EncryptedEnvelope>(rawObject)
        MobileCrypto.decryptJsonObject(envelope, token)
    } catch (e: Exception) {
        throw IllegalStateException("Failed to decrypt Web host SSE event", e)
    }
    return try {
        validateControlTrace(eventObject)
        decodeCoreProjectionHint(eventObject) ?: json.decodeFromJsonElement<SseEventDto>(eventObject).toDomain(eventObject)
    } catch (e: Exception) {
        throw IllegalStateException("Failed to decode Web host SSE event", e)
    }
}

/** Current Host control publications are not assistant text or permission grants. */
private fun decodeCoreProjectionHint(envelope: kotlinx.serialization.json.JsonObject): SseEvent.CoreProjectionHint? {
    val type = envelope["type"]?.jsonPrimitive?.contentOrNull ?: return null
    val payload = when (type) {
        "core_task_updated" -> "task"
        "core_skill_runtime_updated" -> "update"
        "core_capabilities_activated" -> "capability_ids"
        "core_permission_projection_updated" -> "permission_projection"
        "core_memory_runtime_projection_updated" -> "memory_runtime_projection"
        else -> return null
    }
    require(envelope["protocol_version"]?.jsonPrimitive?.contentOrNull == "1")
    val conversation = requireNotNull(envelope["session_id"]?.jsonPrimitive?.contentOrNull).also { require(it.isNotBlank()) }
    if (type == "core_capabilities_activated") {
        require(envelope[payload] is kotlinx.serialization.json.JsonArray)
    } else require(envelope[payload] is kotlinx.serialization.json.JsonObject)
    return SseEvent.CoreProjectionHint(conversation, type)
}
