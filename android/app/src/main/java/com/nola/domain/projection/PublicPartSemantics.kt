package com.nola.domain.projection

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.longOrNull

/** Read-only display facts shared by public projections and rich Core Parts. Not grants or read handles. */
enum class TextReviewState { Pending, Approved, Rejected, Interrupted }

/** Historical provenance only; these identities grant no execution authority. */
sealed interface ToolExecutionSource {
    data object Model : ToolExecutionSource
    data class CodeCell(val cell: CodeCellIdentity) : ToolExecutionSource
}

data class CodeCellIdentity(
    val hostId: String, val conversationId: String, val turnId: String,
    val parentCallId: String, val cellId: String, val generation: String,
)

data class CapabilityOrigin(
    val kind: String = "unknown",
    val providerId: String? = null,
    val providerTitle: String? = null,
    val namespace: String? = null,
)

data class ToolPermissionDisplay(
    val requestId: String,
    val permissionDomain: String,
    val rationale: String,
    val decision: String? = null,
)

data class ToolOutputOwner(val messageId: String, val partId: String, val toolCallId: String)
data class PartArtifact(val uri: String, val name: String, val mimeType: String? = null, val operation: String? = null)
data class SourceLocation(
    val page: ULong? = null,
    val lineStart: ULong? = null,
    val lineEnd: ULong? = null,
    val section: String? = null,
)
data class CitationRange(val start: ULong, val end: ULong)
data class MemoryNamespace(val kind: String, val name: String? = null)
data class MemoryUsageEvidence(
    val recordId: String,
    val componentId: String,
    val scope: String? = null,
    val namespace: MemoryNamespace? = null,
    val provenanceKind: String? = null,
    val source: String? = null,
    val title: String? = null,
    val observedAt: String? = null,
    val sourceRole: String? = null,
    val eventTime: String? = null,
    val rangeStart: String? = null,
    val rangeEnd: String? = null,
    val requiresCurrentStateVerification: Boolean = false,
    val truncated: Boolean = false,
    val supersedes: List<String> = emptyList(),
    val usageCount: ULong = 0uL,
    val lastUsedAtEpochMs: Long? = null,
)
data class SourceEvidence(val kind: String, val usage: MemoryUsageEvidence)

/** Reject authority/private fields rather than retaining opaque objects in public display models. */
object PublicPartSemanticsDecoder {
    fun executionSource(value: JsonElement): ToolExecutionSource {
        val obj = value.strictObject("execution source", setOf("kind", "cell"))
        return when (obj.string("kind")) {
            "model" -> {
                require(obj.keys == setOf("kind")) { "Model source cannot carry cell identity" }
                ToolExecutionSource.Model
            }
            "code_cell" -> {
                val fields = setOf("host_id", "conversation_id", "turn_id", "parent_call_id", "cell_id", "generation")
                val cell = obj.getValue("cell").strictObject("code cell identity", fields)
                require(cell.keys == fields) { "Incomplete code cell identity" }
                fun id(key: String): String = cell.string(key).also {
                    require(it.isNotBlank() && it.toByteArray(Charsets.UTF_8).size <= 512 &&
                        it.none { character -> Character.isISOControl(character) }) { "Invalid code cell identity" }
                }
                ToolExecutionSource.CodeCell(CodeCellIdentity(id("host_id"), id("conversation_id"), id("turn_id"),
                    id("parent_call_id"), id("cell_id"), id("generation")))
            }
            else -> error("Unknown tool execution source")
        }
    }

    fun review(value: JsonElement?): TextReviewState? = value.present()?.let {
        when (it.string("review")) {
            "pending" -> TextReviewState.Pending
            "approved" -> TextReviewState.Approved
            "rejected" -> TextReviewState.Rejected
            "interrupted" -> TextReviewState.Interrupted
            else -> error("Unknown text review")
        }
    }

    fun origin(value: JsonElement): CapabilityOrigin {
        val obj = value.strictObject("origin", setOf("kind", "provider_id", "provider_title", "namespace"))
        val kind = if ("kind" in obj) obj.string("kind") else "unknown"
        require(kind in setOf("unknown", "builtin", "skill", "mcp", "device", "extension", "internal"))
        return CapabilityOrigin(kind, obj.optionalString("provider_id"), obj.optionalString("provider_title"), obj.optionalString("namespace"))
    }

    fun permission(value: JsonElement?): ToolPermissionDisplay? = value.present()?.let {
        val obj = it.strictObject("permission", setOf("request_id", "permission_domain", "rationale", "decision"))
        ToolPermissionDisplay(obj.string("request_id"), obj.string("permission_domain"), obj.string("rationale"), obj.optionalString("decision"))
    }

    fun outputOwner(value: JsonElement?): ToolOutputOwner? = value.present()?.let {
        val obj = it.strictObject("full_output", setOf("message_id", "part_id", "tool_call_id"))
        ToolOutputOwner(obj.string("message_id"), obj.string("part_id"), obj.string("tool_call_id"))
    }

    fun artifact(value: JsonElement): PartArtifact {
        val obj = value.strictObject("artifact", setOf("uri", "name", "mime_type", "operation"))
        return PartArtifact(obj.string("uri"), obj.string("name"), obj.optionalString("mime_type"), obj.optionalString("operation"))
    }

    fun location(value: JsonElement?): SourceLocation? = value.present()?.let {
        val obj = it.strictObject("location", setOf("page", "line_start", "line_end", "section"))
        SourceLocation(unsigned(obj["page"]), unsigned(obj["line_start"]), unsigned(obj["line_end"]), obj.optionalString("section"))
    }

    fun range(value: JsonElement?): CitationRange? = value.present()?.let {
        val obj = it.strictObject("range", setOf("start", "end"))
        CitationRange(unsigned(obj.getValue("start")) ?: error("Missing range start"), unsigned(obj.getValue("end")) ?: error("Missing range end"))
    }

    fun evidence(value: JsonElement?): SourceEvidence? = value.present()?.let {
        val obj = it.strictObject("evidence", setOf("kind", "usage"))
        require(obj.string("kind") == "memory") { "Unknown source evidence" }
        val usage = obj.getValue("usage").strictObject("memory usage", setOf(
            "record_id", "component_id", "scope", "namespace", "provenance_kind", "source", "title", "observed_at",
            "source_role", "event_time", "range_start", "range_end", "requires_current_state_verification", "truncated",
            "supersedes", "usage_count", "last_used_at_epoch_ms",
        ))
        SourceEvidence("memory", MemoryUsageEvidence(
            recordId = usage.string("record_id"), componentId = usage.string("component_id"),
            scope = usage.enumString("scope", setOf("turn", "conversation", "global")),
            namespace = namespace(usage["namespace"]),
            provenanceKind = usage.enumString("provenance_kind", setOf("explicit_user", "agent_inference", "tool_result", "imported", "synced", "other")),
            source = usage.optionalString("source"), title = usage.optionalString("title"), observedAt = usage.optionalString("observed_at"),
            sourceRole = usage.enumString("source_role", setOf("user", "assistant", "tool", "system", "other")),
            eventTime = usage.optionalString("event_time"), rangeStart = usage.optionalString("range_start"), rangeEnd = usage.optionalString("range_end"),
            requiresCurrentStateVerification = usage.optionalBoolean("requires_current_state_verification"),
            truncated = usage.optionalBoolean("truncated"),
            supersedes = usage["supersedes"]?.let { list -> (list as? JsonArray ?: error("supersedes must be an array")).map { item -> item.string("supersedes") } }.orEmpty(),
            usageCount = if ("usage_count" in usage) unsigned(usage.getValue("usage_count"))
                ?: error("usage_count cannot be null") else 0uL,
            lastUsedAtEpochMs = usage["last_used_at_epoch_ms"].present()?.let { signed ->
                (signed as? JsonPrimitive)?.takeUnless { primitive -> primitive.isString }?.longOrNull ?: error("Invalid signed epoch")
            },
        ))
    }

    fun unsigned(value: JsonElement?): ULong? = value.present()?.let {
        val primitive = it as? JsonPrimitive ?: error("Expected unsigned integer")
        require(!primitive.isString) { "Unsigned integer cannot be a string" }
        require(primitive.content.isNotEmpty() && primitive.content.all { digit -> digit in '0'..'9' } &&
            (primitive.content.length == 1 || primitive.content.first() != '0')) { "Invalid unsigned integer syntax" }
        primitive.content.toULongOrNull() ?: error("Unsigned integer out of range")
    }

    private fun namespace(value: JsonElement?): MemoryNamespace? = value.present()?.let {
        val obj = it as? JsonObject ?: error("namespace must be an object")
        val kind = obj.string("kind")
        when (kind) {
            "personal", "workspace", "conversation" -> { obj.checkFields("namespace", setOf("kind")); MemoryNamespace(kind) }
            "custom" -> { obj.checkFields("namespace", setOf("kind", "name")); MemoryNamespace(kind, obj.string("name")) }
            else -> error("Unknown memory namespace")
        }
    }
}

private fun JsonElement?.present(): JsonElement? = this?.takeUnless { it is JsonNull }
private fun JsonElement.string(label: String): String = (this as? JsonPrimitive)?.takeIf { it.isString }?.content
    ?: error("$label must be a string")
private fun JsonObject.string(key: String): String = getValue(key).string(key)
private fun JsonObject.optionalString(key: String): String? = this[key].present()?.string(key)
private fun JsonObject.enumString(key: String, values: Set<String>): String? = optionalString(key)?.also { require(it in values) { "Unknown $key" } }
private fun JsonObject.optionalBoolean(key: String): Boolean = this[key]?.let {
    (it as? JsonPrimitive)?.takeUnless { primitive -> primitive.isString }?.booleanOrNull ?: error("Invalid $key")
} ?: false
private fun JsonElement.strictObject(label: String, fields: Set<String>): JsonObject = (this as? JsonObject
    ?: error("$label must be an object")).also { it.checkFields(label, fields) }
private fun JsonObject.checkFields(label: String, fields: Set<String>) { require(keys.all { it in fields }) { "Unknown $label field" } }

/** Legacy Long-only metadata adapters must omit unrepresentable facts, never round or wrap them. */
fun ULong.asLegacyLongOrNull(): Long? = takeIf { it <= Long.MAX_VALUE.toULong() }?.toLong()
