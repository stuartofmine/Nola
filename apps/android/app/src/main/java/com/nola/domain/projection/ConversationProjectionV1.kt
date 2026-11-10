package com.nola.domain.projection

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long

/**
 * Android's typed read contract for the canonical conversation projection.
 *
 * This is deliberately not a Store-row model and not a Host-event model.
 * Unknown enum values and Part kinds fail at the boundary so the UI cannot
 * silently infer a plausible but incorrect conversation state.
 */
data class ConversationProjectionV1(
    val contractVersion: Int,
    val conversationId: String,
    val revision: Long,
    val title: String,
    val messages: List<Message>,
    val activeRun: ActiveRun?,
    val contextUsage: ContextUsage,
    val surface: Surface = Surface(),
    val taskChildOwner: TaskChildOwner? = null,
) {
    data class TaskChildOwner(val parentConversationId: String, val parentRunId: String, val taskId: String)

    data class Message(
        val messageId: String,
        val role: Role,
        val state: MessageState,
        val revision: Long,
        val createdAtMs: Long,
        val parts: List<Part>,
    )

    enum class Role { User, Assistant, System }
    enum class MessageState { Pending, Running, Completed, Cancelled, Failed }
    enum class ActivityState { Pending, Running, WaitingForPermission, WaitingForInput, Completed, Sealed, Cancelled, Failed }

    sealed interface Part {
        val partId: String
        val revision: Long

        data class UiAction(
            override val partId: String,
            override val revision: Long,
            val surface: com.nola.data.local.UiSurfaceRevisionRef,
            val envelope: JsonObject,
            val summary: String,
        ) : Part

        data class UiSurface(
            override val partId: String,
            override val revision: Long,
            val surface: com.nola.data.local.UiSurfaceRevisionRef,
            val summary: String,
        ) : Part

        // Historical payloads are data only; they never become Tool or permission requests.
        data class Historical(
            override val partId: String,
            override val revision: Long,
            val projection: HistoricalProjection,
        ) : Part

        data class Text(
            override val partId: String,
            override val revision: Long,
            val markdown: String,
            val isFinal: Boolean,
            val review: TextReviewState? = null,
        ) : Part {
            val disclosedMarkdown: String get() = if (review == null || review == TextReviewState.Approved) markdown else ""
        }

        data class Reasoning(
            override val partId: String,
            override val revision: Long,
            val state: ActivityState,
            val summary: String?,
            val title: String = "",
        ) : Part

        data class Tool(
            override val partId: String,
            override val revision: Long,
            val toolCallId: String,
            val capabilityId: String,
            val executionSource: ToolExecutionSource,
            val state: ActivityState,
            val displayKey: String?,
            val input: JsonElement,
            val output: JsonElement?,
            val affectedResources: List<AffectedResource>,
            val title: String = "",
            val summary: String? = null,
            val origin: CapabilityOrigin = CapabilityOrigin(),
            val permission: ToolPermissionDisplay? = null,
            val outputPreview: String? = null,
            val fullOutput: ToolOutputOwner? = null,
            val artifacts: List<PartArtifact> = emptyList(),
            val startedAtMs: ULong? = null,
            val finishedAtMs: ULong? = null,
        ) : Part

        data class Source(
            override val partId: String,
            override val revision: Long,
            val sourceId: String,
            val title: String,
            val uri: String,
            val mimeType: String?,
            val snippet: String?,
            val evidence: SourceEvidence? = null,
        ) : Part

        data class Citation(
            override val partId: String,
            override val revision: Long,
            val sourceId: String,
            val title: String,
            val uri: String,
            val originToolCallId: String,
            val page: ULong?,
            val lineStart: ULong?,
            val lineEnd: ULong?,
            val rangeStart: ULong?,
            val rangeEnd: ULong?,
            val citationId: String = "",
            val mimeType: String? = null,
            val location: SourceLocation? = null,
            val section: String? = null,
        ) : Part {
            val range: CitationRange? get() = if (rangeStart != null && rangeEnd != null) CitationRange(rangeStart, rangeEnd) else null
        }

        data class Attachment(
            override val partId: String,
            override val revision: Long,
            val attachmentId: String,
            val name: String,
            val mimeType: String?,
            val uri: String,
            val operation: String? = null,
        ) : Part

        data class Task(
            override val partId: String,
            override val revision: Long,
            val taskId: String,
            val displayKey: String,
            val state: ActivityState,
            val progress: Double?,
            val currentStep: String?,
            val resultRef: String?,
            val cancelRequested: Boolean,
            val supportsGuidance: Boolean = false,
            val supportsCancellation: Boolean = false,
            val parentRunId: String? = null,
        ) : Part

        data class Skill(
            override val partId: String,
            override val revision: Long,
            val runId: String,
            val skillId: String,
            val displayName: String,
            val mode: String,
            val state: ActivityState,
            val modelTurns: Long,
            val toolCalls: Long,
            val elapsedMillis: Long,
            val resultRef: String?,
            val failure: String?,
        ) : Part

        data class ModelAttemptFailure(
            val code: String,
            val message: String,
            val retryable: Boolean,
        )

        data class ModelAttempt(
            override val partId: String,
            override val revision: Long,
            val state: ActivityState,
            val attemptId: String,
            val provider: String?,
            val model: String?,
            val failure: ModelAttemptFailure,
            val fallbackAttemptId: String?,
            val fallbackProvider: String?,
            val fallbackModel: String?,
        ) : Part

        data class Compaction(
            override val partId: String,
            override val revision: Long,
            val summary: String,
            val sourceItemIds: List<String>,
            val sourceFingerprint: String,
            val originalTokenCount: Long,
            val compactedTokenCount: Long,
        ) : Part

        data class Error(
            override val partId: String,
            override val revision: Long,
            val code: String,
            val displayKey: String,
            val recoverable: Boolean,
        ) : Part
    }

    data class HistoricalProjection(
        val owner: JsonObject,
        val canonicalRevision: Long,
        val source: JsonElement?,
        val schema: String?,
        val canonicalKind: String,
        val binding: String?,
        val status: HistoricalStatus,
        val unsupportedCode: String?,
        val fields: Map<String, HistoricalField>,
        val canonicalText: String? = null,
        val canonicalTextUTF8Bytes: Long? = null,
        val notLoadedReason: String? = null,
    ) {
        val decodedText: String? get() {
            if (status == HistoricalStatus.Unsupported || status == HistoricalStatus.NotLoaded) return null
            val key = when (schema to canonicalKind) {
                "agent_part" to "text" -> "text"
                "message_part_view" to "text" -> "markdown"
                "agent_part" to "reasoning" -> "detail"
                "message_part_view" to "reasoning" -> "summary"
                else -> return null
            }
            val value = (fields[key] as? HistoricalField.Complete)?.value
            return (value as? kotlinx.serialization.json.JsonPrimitive)?.takeIf { it.isString }?.content
        }
        /** Content suitable for the chat timeline; wire diagnostics stay in details. */
        val timelineText: String? get() = if (canonicalKind == "reasoning") null else decodedText ?: canonicalText
        val statusLabel: String get() = "[Historical content · $canonicalKind · ${status.name.lowercase()}" +
            ((unsupportedCode ?: notLoadedReason)?.let { " · $it" } ?: "") + "]" + (canonicalText?.let { "\n$it" } ?: "")
    }
    enum class HistoricalStatus { Supported, Deferred, Unsupported, NotLoaded }
    sealed interface HistoricalField {
        data class Complete(val value: JsonElement) : HistoricalField
        data class Deferred(val locator: JsonObject) : HistoricalField
    }

    data class AffectedResource(
        val resourceType: String,
        val uri: String,
        val operation: String,
    )

    data class ActiveRun(
        val runId: String,
        val state: ActivityState,
        val activePartId: String?,
        val displayKey: String?,
        val progress: Double?,
    )

    data class ContextUsage(
        val usedTokens: Long,
        val windowTokens: Long,
        val promptTokenLimit: Long,
        val estimated: Boolean,
        val measuredAtRevision: Long,
        val measurement: JsonObject? = null,
        val promptCacheMeasurement: JsonObject? = null,
        val promptCacheDiagnostic: JsonObject? = null,
        val conversationCacheUsage: JsonObject? = null,
    )

    data class Surface(
        val revision: Long = 0,
        val committedMessageIds: List<String> = emptyList(),
        val liveMessageId: String? = null,
        val currentActivityPartId: String? = null,
        val archivedActivityPartIds: List<String> = emptyList(),
        val showAssistantPlaceholder: Boolean = false,
    )
}

object ConversationProjectionV1Decoder {
    fun decodeDisplayPart(value: JsonElement, messageId: String): ConversationProjectionV1.Part = decodePart(value, messageId)
    private const val CONTRACT_VERSION = 1

    fun decode(value: JsonElement): ConversationProjectionV1 {
        val root = value.requiredObject("conversation projection")
        val version = root.requiredLong("contract_version").toInt()
        require(version == CONTRACT_VERSION) {
            "Unsupported conversation projection contract version: $version"
        }
        val messages = root.requiredArray("messages").map(::decodeMessage)
        val conversationId = root.requiredString("conversation_id")
        require(messages.flatMap { it.parts }.filterIsInstance<ConversationProjectionV1.Part.UiAction>()
            .all { it.surface.identity.owner.conversation_id == conversationId })
        require(messages.flatMap { it.parts }.filterIsInstance<ConversationProjectionV1.Part.UiSurface>()
            .all { it.surface.identity.owner.conversation_id == conversationId }) { "UI surface conversation mismatch" }
        val activeRun = root.optionalObject("active_run")?.let(::decodeActiveRun)
        return ConversationProjectionV1(
            taskChildOwner = decodeTaskChildOwner(root, conversationId),
            contractVersion = version,
            conversationId = root.requiredString("conversation_id"),
            revision = root.requiredLong("revision"),
            title = root.requiredString("title"),
            messages = messages,
            activeRun = activeRun,
            contextUsage = decodeContextUsage(root.requiredObject("context_usage")),
            surface = root.optionalObject("surface")?.let(::decodeSurface)
                ?: deriveSurface(root.requiredLong("revision"), messages, activeRun),
        )
    }

    private fun decodeTaskChildOwner(root: JsonObject, conversationId: String): ConversationProjectionV1.TaskChildOwner? {
        val owner = root.requiredObject("execution_ownership")
        return when (owner.requiredString("kind")) {
            "user" -> null
            "task_child" -> {
                require(owner.requiredString("child_conversation_id") == conversationId) { "Child ownership mismatch" }
                ConversationProjectionV1.TaskChildOwner(owner.requiredString("parent_conversation_id"),
                    owner.requiredString("parent_run_id"), owner.requiredString("task_id"))
            }
            else -> error("Unsupported conversation ownership")
        }
    }

    fun decodeWindow(value: JsonElement): ConversationProjectionWindowV1 {
        val root = value.requiredObject("conversation projection window")
        return ConversationProjectionWindowV1(
            projection = decode(root.getValue("projection")),
            eventSequence = root.requiredLong("event_sequence"),
            totalMessageCount = root.requiredLong("total_message_count"),
            beforeOrdinal = root.optionalLong("before_ordinal"),
            hasMore = root.requiredBoolean("has_more"),
            assistantJournalHeads = root["assistant_journal_heads"]?.jsonArray?.map { it.jsonObject }.orEmpty(),
        )
    }

    private fun decodeMessage(value: JsonElement): ConversationProjectionV1.Message {
        val objectValue = value.requiredObject("conversation message")
        val messageId = objectValue.requiredString("message_id")
        return ConversationProjectionV1.Message(
            messageId = messageId,
            role = when (val role = objectValue.requiredString("role")) {
                "user" -> ConversationProjectionV1.Role.User
                "assistant" -> ConversationProjectionV1.Role.Assistant
                "system" -> ConversationProjectionV1.Role.System
                else -> error("Unknown conversation message role: $role")
            },
            state = decodeMessageState(objectValue.requiredString("state")),
            revision = objectValue.requiredLong("revision"),
            createdAtMs = objectValue.requiredLong("created_at_ms"),
            parts = objectValue.requiredArray("parts").map { decodePart(it, messageId) },
        )
    }

    fun decodeHistoricalProjection(value: JsonElement): ConversationProjectionV1.HistoricalProjection {
        val historical = value.requiredObject("historical projection")
        val status = historical.getValue("status").requiredObject("historical status")
        val state = when (status.requiredString("state")) {
            "not_loaded" -> ConversationProjectionV1.HistoricalStatus.NotLoaded
            "supported" -> ConversationProjectionV1.HistoricalStatus.Supported
            "deferred" -> ConversationProjectionV1.HistoricalStatus.Deferred
            "unsupported" -> ConversationProjectionV1.HistoricalStatus.Unsupported
            else -> error("Unknown historical status")
        }
        val fields = historical.getValue("fields").requiredObject("historical fields").mapValues { (_, field) ->
            val payload = field.requiredObject("historical field")
            when (payload.requiredString("availability")) {
                "complete" -> ConversationProjectionV1.HistoricalField.Complete(payload.getValue("value"))
                "deferred" -> ConversationProjectionV1.HistoricalField.Deferred(payload.getValue("locator").requiredObject("locator"))
                else -> error("Unknown historical availability")
            }
        }
        return ConversationProjectionV1.HistoricalProjection(
                historical.getValue("owner").requiredObject("historical owner"),
                historical.requiredLong("canonical_revision"), historical["source"],
                historical.optionalString("schema"), historical.requiredString("canonical_kind"),
                historical.optionalString("binding"), state,
                if (state == ConversationProjectionV1.HistoricalStatus.Unsupported) status.requiredString("code") else null,
                fields,
                historical.optionalObject("canonical_text")?.requiredString("text"),
                historical.optionalObject("canonical_text")?.requiredLong("total_utf8_bytes"),
                status.optionalString("reason"),
            )
    }

    private fun decodePart(value: JsonElement, messageId: String): ConversationProjectionV1.Part {
        val part = value.requiredObject("conversation Part")
        val partId = part.requiredString("part_id")
        val revision = part.requiredLong("revision")
        return when (val kind = part.requiredString("kind")) {
            "historical" -> ConversationProjectionV1.Part.Historical(partId, revision, decodeHistoricalProjection(part.getValue("projection")))
            "text" -> {
                part.checkPublicFields(setOf("markdown", "is_final", "review"))
                val text = ConversationProjectionV1.Part.Text(
                    partId, revision, part.requiredString("markdown"), part.requiredBoolean("is_final"),
                    PublicPartSemanticsDecoder.review(part["review"]),
                )
                require(text.markdown == text.disclosedMarkdown) { "Withheld review cannot carry public text" }
                text
            }
            "reasoning" -> {
                part.checkPublicFields(setOf("state", "summary", "title"))
                ConversationProjectionV1.Part.Reasoning(partId, revision, decodeActivityState(part.requiredString("state")),
                    part.optionalString("summary"), part.requiredString("title"))
            }
            "ui_action" -> {
                part.checkPublicFields(setOf("surface", "envelope", "summary"))
                val reference = kotlinx.serialization.json.Json.decodeFromString(
                    com.nola.data.local.UiSurfaceRevisionRef.serializer(), part.getValue("surface").toString())
                require(reference.revision > 0 && reference.identity.generation > 0)
                val owner = reference.identity.owner
                require(listOf(owner.authority_id, owner.conversation_id, owner.participant_id, owner.context_id,
                    owner.originating_run_id, owner.message_id, owner.part_id, reference.identity.surface_id).all { it.isNotBlank() })
                ConversationProjectionV1.Part.UiAction(partId, revision, reference,
                    part.getValue("envelope").jsonObject, part.requiredString("summary"))
            }
            "ui_surface" -> {
                part.checkPublicFields(setOf("surface", "summary"))
                val reference = kotlinx.serialization.json.Json.decodeFromString(
                    com.nola.data.local.UiSurfaceRevisionRef.serializer(), part.getValue("surface").toString())
                val owner = reference.identity.owner
                require(reference.revision > 0 && reference.identity.generation > 0 && owner.message_id == messageId)
                require(listOf(owner.authority_id, owner.conversation_id, owner.participant_id, owner.context_id,
                    owner.originating_run_id, owner.message_id, owner.part_id, reference.identity.surface_id).all { it.isNotBlank() })
                ConversationProjectionV1.Part.UiSurface(partId, revision, reference, part.requiredString("summary"))
            }
            "tool" -> {
                part.checkPublicFields(setOf("execution_source", "tool_call_id", "capability_id", "state", "display_key", "title", "summary", "origin", "permission", "input", "output", "output_preview", "full_output", "artifacts", "started_at_ms", "finished_at_ms", "affected_resources"))
                val capabilityId = part.requiredString("capability_id")
                require(capabilityId.isNotBlank()) { "Tool Part $partId has an empty capability_id" }
                val callId = part.requiredString("tool_call_id")
                val outputOwner = PublicPartSemanticsDecoder.outputOwner(part["full_output"])
                require(outputOwner == null || outputOwner == ToolOutputOwner(messageId, partId, callId)) { "Tool output owner mismatch" }
                ConversationProjectionV1.Part.Tool(
                    partId = partId,
                    revision = revision,
                    toolCallId = callId,
                    capabilityId = capabilityId,
                    state = decodeActivityState(part.requiredString("state")),
                    displayKey = part.optionalString("display_key"),
                    input = part.getValue("input"),
                    output = part["output"],
                    affectedResources = part.requiredArray("affected_resources").map { resource ->
                        val item = resource.requiredObject("affected resource")
                        ConversationProjectionV1.AffectedResource(
                            item.requiredString("resource_type"),
                            item.requiredString("uri"),
                            item.requiredString("operation"),
                        )
                    },
                    title = part.requiredString("title"), summary = part.optionalString("summary"),
                    origin = PublicPartSemanticsDecoder.origin(part.getValue("origin")),
                    executionSource = PublicPartSemanticsDecoder.executionSource(part.getValue("execution_source")),
                    permission = PublicPartSemanticsDecoder.permission(part["permission"]),
                    outputPreview = part.optionalString("output_preview"), fullOutput = outputOwner,
                    artifacts = part.requiredArray("artifacts").map(PublicPartSemanticsDecoder::artifact),
                    startedAtMs = PublicPartSemanticsDecoder.unsigned(part["started_at_ms"]),
                    finishedAtMs = PublicPartSemanticsDecoder.unsigned(part["finished_at_ms"]),
                )
            }
            "source" -> {
                part.checkPublicFields(setOf("source_id", "title", "uri", "mime_type", "snippet", "evidence"))
                ConversationProjectionV1.Part.Source(partId, revision, part.requiredString("source_id"), part.requiredString("title"),
                    part.requiredString("uri"), part.optionalString("mime_type"), part.optionalString("snippet"),
                    PublicPartSemanticsDecoder.evidence(part["evidence"]))
            }
            "citation" -> {
                part.checkPublicFields(setOf("citation_id", "source_id", "title", "uri", "mime_type", "origin_tool_call_id", "location", "section", "page", "line_start", "line_end", "range_start", "range_end"))
                val location = PublicPartSemanticsDecoder.location(part["location"])
                val citation = ConversationProjectionV1.Part.Citation(
                    partId, revision, part.requiredString("source_id"), part.requiredString("title"),
                    part.requiredString("uri"), part.requiredString("origin_tool_call_id"),
                    PublicPartSemanticsDecoder.unsigned(part["page"]), PublicPartSemanticsDecoder.unsigned(part["line_start"]), PublicPartSemanticsDecoder.unsigned(part["line_end"]),
                    PublicPartSemanticsDecoder.unsigned(part["range_start"]), PublicPartSemanticsDecoder.unsigned(part["range_end"]),
                    part.requiredString("citation_id"), part.optionalString("mime_type"), location, part.optionalString("section"),
                )
                require(citation.page == location?.page && citation.lineStart == location?.lineStart && citation.lineEnd == location?.lineEnd && citation.section == location?.section) { "Citation location mismatch" }
                require((citation.rangeStart == null) == (citation.rangeEnd == null)) { "Incomplete citation range" }
                citation
            }
            "attachment" -> {
                part.checkPublicFields(setOf("attachment_id", "name", "mime_type", "uri", "operation"))
                ConversationProjectionV1.Part.Attachment(partId, revision, part.requiredString("attachment_id"), part.requiredString("name"),
                    part.optionalString("mime_type"), part.requiredString("uri"), part.optionalString("operation"))
            }
            "task", "task_control" -> ConversationProjectionV1.Part.Task(
                partId, revision, part.requiredString("task_id"), part.requiredString("display_key"),
                decodeActivityState(part.requiredString("state")), part.optionalDouble("progress"),
                part.optionalString("current_step"), part.optionalString("result_ref"),
                part.requiredBoolean("cancel_requested"),
                if (part.requiredString("kind") == "task_control") part.requiredBoolean("supports_guidance") else false,
                if (part.requiredString("kind") == "task_control") part.requiredBoolean("supports_cancellation") else false,
                part.optionalString("parent_run_id"),
            )
            "skill" -> ConversationProjectionV1.Part.Skill(
                partId, revision, part.requiredString("run_id"), part.requiredString("skill_id"),
                part.requiredString("display_name"), part.requiredString("mode"),
                decodeActivityState(part.requiredString("state")), part.requiredLong("model_turns"),
                part.requiredLong("tool_calls"), part.requiredLong("elapsed_millis"),
                part.optionalString("result_ref"), part.optionalString("failure")
            )
            "model_attempt" -> {
                val failure = part.getValue("failure").requiredObject("model attempt failure")
                ConversationProjectionV1.Part.ModelAttempt(
                    partId, revision, decodeActivityState(part.requiredString("state")),
                    part.requiredString("attempt_id"), part.optionalString("provider"),
                    part.optionalString("model"), ConversationProjectionV1.Part.ModelAttemptFailure(
                        failure.requiredString("code"), failure.requiredString("message"),
                        failure.requiredBoolean("retryable"),
                    ), part.optionalString("fallback_attempt_id"),
                    part.optionalString("fallback_provider"), part.optionalString("fallback_model"),
                )
            }
            "compaction" -> ConversationProjectionV1.Part.Compaction(
                partId, revision, part.requiredString("summary"),
                (if ("source_item_ids" in part) {
                    part.requiredArray("source_item_ids")
                } else {
                    part.requiredArray("retained_item_ids")
                }).map { it.jsonPrimitive.content },
                part.optionalString("source_fingerprint").orEmpty(),
                part.requiredLong("original_token_count"), part.requiredLong("compacted_token_count")
            )
            "error" -> ConversationProjectionV1.Part.Error(
                partId, revision, part.requiredString("code"), part.requiredString("display_key"),
                part.requiredBoolean("recoverable")
            )
            else -> error("Unknown conversation Part kind: $kind")
        }
    }

    private fun decodeActiveRun(value: JsonObject) = ConversationProjectionV1.ActiveRun(
        runId = value.requiredString("run_id"),
        state = decodeActivityState(value.requiredString("state")),
        activePartId = value.optionalString("active_part_id"),
        displayKey = value.optionalString("display_key"),
        progress = value.optionalDouble("progress"),
    )

    fun decodeContextUsage(value: JsonObject) = ConversationProjectionV1.ContextUsage(
        usedTokens = value.requiredLong("used_tokens"),
        windowTokens = value.requiredLong("context_window_tokens"),
        promptTokenLimit = value.requiredLong("prompt_token_limit"),
        estimated = value.requiredBoolean("estimated"),
        measuredAtRevision = value.requiredLong("measured_at_revision"),
        measurement = contextMeasurement(value["measurement"]),
        promptCacheMeasurement = contextMeasurement(value["prompt_cache_measurement"]),
        promptCacheDiagnostic = value["prompt_cache_diagnostic"]?.takeUnless { it == JsonNull }?.jsonObject?.also {
            require(value["prompt_cache_measurement"] != null && value["prompt_cache_measurement"] != JsonNull) { "Cache diagnostic has no source request" }
        },
        conversationCacheUsage = value["conversation_cache_usage"]?.takeUnless { it == JsonNull }?.jsonObject,
    )

    private fun contextMeasurement(value: JsonElement?): JsonObject? = value?.takeUnless { it == JsonNull }?.jsonObject?.also {
        require(it.keys == setOf("request_id", "attempt_id", "context_generation") &&
            it.requiredString("request_id").isNotBlank() && it.optionalString("attempt_id")?.isBlank() != true)
    }

    private fun decodeSurface(value: JsonObject) = ConversationProjectionV1.Surface(
        revision = value.requiredLong("revision"),
        committedMessageIds = value.requiredArray("committed_message_ids")
            .map { it.jsonPrimitive.content },
        liveMessageId = value.optionalString("live_message_id"),
        currentActivityPartId = value.optionalString("current_activity_part_id"),
        archivedActivityPartIds = value.requiredArray("archived_activity_part_ids")
            .map { it.jsonPrimitive.content },
        showAssistantPlaceholder = value.requiredBoolean("show_assistant_placeholder"),
    )

    private fun deriveSurface(
        revision: Long,
        messages: List<ConversationProjectionV1.Message>,
        activeRun: ConversationProjectionV1.ActiveRun?,
    ): ConversationProjectionV1.Surface {
        val liveMessage = messages.lastOrNull { message ->
            message.role == ConversationProjectionV1.Role.Assistant &&
                message.state in setOf(
                    ConversationProjectionV1.MessageState.Pending,
                    ConversationProjectionV1.MessageState.Running,
                )
        }
        val archivedActivityIds = messages.flatMap { message ->
            message.parts.mapNotNull { part ->
                val terminal = when (part) {
                    is ConversationProjectionV1.Part.Reasoning -> part.state
                    is ConversationProjectionV1.Part.Tool -> part.state
                    is ConversationProjectionV1.Part.Task -> part.state
                    is ConversationProjectionV1.Part.Skill -> part.state
                    is ConversationProjectionV1.Part.ModelAttempt -> part.state
                    else -> null
                }
                part.partId.takeIf {
                    terminal in setOf(
                        ConversationProjectionV1.ActivityState.Completed,
                        ConversationProjectionV1.ActivityState.Sealed,
                        ConversationProjectionV1.ActivityState.Cancelled,
                        ConversationProjectionV1.ActivityState.Failed,
                    )
                }
            }
        }
        return ConversationProjectionV1.Surface(
            revision = revision,
            committedMessageIds = messages
                .filterNot { it.messageId == liveMessage?.messageId }
                .map { it.messageId },
            liveMessageId = liveMessage?.messageId,
            currentActivityPartId = activeRun?.activePartId,
            archivedActivityPartIds = archivedActivityIds,
            showAssistantPlaceholder = liveMessage?.parts?.none {
                it is ConversationProjectionV1.Part.Text && it.disclosedMarkdown.isNotBlank()
            } == true,
        )
    }

    private fun decodeMessageState(value: String) = when (value) {
        "pending" -> ConversationProjectionV1.MessageState.Pending
        "running" -> ConversationProjectionV1.MessageState.Running
        "completed" -> ConversationProjectionV1.MessageState.Completed
        "cancelled" -> ConversationProjectionV1.MessageState.Cancelled
        "failed" -> ConversationProjectionV1.MessageState.Failed
        else -> error("Unknown conversation message state: $value")
    }

    private fun decodeActivityState(value: String) = when (value) {
        "pending" -> ConversationProjectionV1.ActivityState.Pending
        "running" -> ConversationProjectionV1.ActivityState.Running
        "waiting_for_permission" -> ConversationProjectionV1.ActivityState.WaitingForPermission
        "waiting_for_input" -> ConversationProjectionV1.ActivityState.WaitingForInput
        "completed" -> ConversationProjectionV1.ActivityState.Completed
        "sealed" -> ConversationProjectionV1.ActivityState.Sealed
        "cancelled" -> ConversationProjectionV1.ActivityState.Cancelled
        "failed" -> ConversationProjectionV1.ActivityState.Failed
        else -> error("Unknown conversation activity state: $value")
    }
}

data class ConversationProjectionWindowV1(
    val projection: ConversationProjectionV1,
    val eventSequence: Long,
    val totalMessageCount: Long,
    val beforeOrdinal: Long?,
    val hasMore: Boolean,
    val assistantJournalHeads: List<JsonObject> = emptyList(),
)

private fun JsonElement.requiredObject(label: String): JsonObject = this as? JsonObject
    ?: error("$label must be an object")

private fun JsonObject.required(key: String): JsonElement = this[key]?.takeUnless { it is JsonNull }
    ?: error("Missing required field: $key")

private fun JsonObject.optional(key: String): JsonElement? = this[key]?.takeUnless { it is JsonNull }
private fun JsonObject.requiredObject(key: String): JsonObject = required(key).jsonObject
private fun JsonObject.optionalObject(key: String): JsonObject? = optional(key)?.jsonObject
private fun JsonObject.requiredArray(key: String): JsonArray = required(key).jsonArray
private fun JsonObject.requiredString(key: String): String = required(key).jsonPrimitive.let {
    require(it.isString) { "$key must be a string" }; it.content
}
private fun JsonObject.optionalString(key: String): String? = optional(key)?.jsonPrimitive?.let {
    require(it.isString) { "$key must be a string" }; it.content
}
private fun JsonObject.requiredLong(key: String): Long = required(key).jsonPrimitive.long
private fun JsonObject.optionalLong(key: String): Long? = optional(key)?.jsonPrimitive?.long
private fun JsonObject.optionalDouble(key: String): Double? = optional(key)?.jsonPrimitive?.double
private fun JsonObject.requiredBoolean(key: String): Boolean = required(key).jsonPrimitive.boolean
private fun JsonObject.checkPublicFields(fields: Set<String>) {
    require(keys.all { it in fields || it in setOf("kind", "part_id", "revision") }) { "Unknown public Part field" }
}
