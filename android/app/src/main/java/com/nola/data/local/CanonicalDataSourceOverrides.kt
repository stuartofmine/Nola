package com.nola.data.local

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

@Serializable
internal enum class CanonicalDataSourceOverride {
    @SerialName("inherit") Inherit,
    @SerialName("enabled") Enabled,
    @SerialName("disabled") Disabled
}

internal class CanonicalDataSourceOverrides(
    private val execute: (JsonElement) -> JsonElement
) {
    fun resolve(conversationId: String, nowMs: Long): List<CanonicalDataSourceSelection> {
        require(conversationId.isNotBlank())
        val result = execute(buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject {
                put("type", "resolve_for_conversation")
                put("conversation_id", conversationId)
                put("now_ms", nowMs)
            })
        })
        return Json.decodeFromJsonElement(ListSerializer(CanonicalDataSourceSelection.serializer()), result)
    }
    fun get(conversationId: String, sourceId: String): CanonicalDataSourceOverride =
        Json.decodeFromJsonElement(CanonicalDataSourceOverride.serializer(), command(
            "get_conversation_override", conversationId, sourceId
        ))

    fun set(
        conversationId: String,
        sourceId: String,
        value: CanonicalDataSourceOverride,
        updatedAtMs: Long
    ) {
        check(command("set_conversation_override", conversationId, sourceId, value, updatedAtMs)
            == JsonPrimitive(true)) { "Application did not acknowledge the data-source override" }
    }

    private fun command(
        type: String,
        conversationId: String,
        sourceId: String,
        value: CanonicalDataSourceOverride? = null,
        updatedAtMs: Long? = null
    ): JsonElement {
        require(conversationId.isNotBlank() && sourceId.isNotBlank())
        return execute(buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject {
                put("type", type)
                put("conversation_id", conversationId)
                put("source_id", sourceId)
                if (value != null) put("value", Json.encodeToJsonElement(CanonicalDataSourceOverride.serializer(), value))
                if (updatedAtMs != null) put("updated_at_ms", updatedAtMs)
            })
        })
    }
}

@Serializable
internal data class CanonicalDataSourceSelection(
    @SerialName("source_id") val sourceId: String,
    @SerialName("source_revision") val sourceRevision: Long,
    val enabled: Boolean,
    val usable: Boolean,
    @SerialName("should_refresh") val shouldRefresh: Boolean,
    val reason: String,
    @SerialName("content_reference") val contentReference: CanonicalSourceContentReference?,
    @SerialName("observed_at_ms") val observedAtMs: Long?,
    @SerialName("schema_version") val schemaVersion: Int?
)

@Serializable
internal data class CanonicalSourceContentReference(
    val uri: String,
    @SerialName("mime_type") val mimeType: String,
    @SerialName("byte_count") val byteCount: Long,
    @SerialName("content_version") val contentVersion: String
)
