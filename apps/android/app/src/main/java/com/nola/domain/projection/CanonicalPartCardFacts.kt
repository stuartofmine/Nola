package com.nola.domain.projection

import kotlinx.serialization.json.JsonElement

/** These are the facts the actual Compose cards consume. No fallback merges independent outputs. */
data class CanonicalToolCardFacts(
    val title: String,
    val capabilityId: String,
    val state: ConversationProjectionV1.ActivityState,
    val summary: String?,
    val input: JsonElement,
    val result: JsonElement?,
    val preview: String?,
    val origin: CapabilityOrigin,
    val permission: ToolPermissionDisplay?,
    val artifacts: List<PartArtifact>,
    val startedAtMs: ULong?,
    val finishedAtMs: ULong?,
    val fullOutput: ToolOutputOwner?,
)

fun ConversationProjectionV1.Part.Tool.cardFacts() = CanonicalToolCardFacts(
    title, capabilityId, state, summary, input, output, outputPreview, origin, permission, artifacts,
    startedAtMs, finishedAtMs, fullOutput,
)

data class CanonicalSourceCardFacts(
    val identity: String,
    val partId: String,
    val sourceId: String,
    val title: String,
    val uri: String,
    val mimeType: String?,
    val location: SourceLocation?,
    val range: CitationRange?,
    val originToolCallId: String?,
    val snippet: String?,
    val evidence: SourceEvidence?,
)

fun ConversationProjectionV1.Part.sourceCardFacts(): CanonicalSourceCardFacts? = when (this) {
    is ConversationProjectionV1.Part.Source -> CanonicalSourceCardFacts(sourceId, partId, sourceId, title, uri, mimeType, null, null, null, snippet, evidence)
    is ConversationProjectionV1.Part.Citation -> CanonicalSourceCardFacts(citationId, partId, sourceId, title, uri, mimeType, location, range, originToolCallId, null, null)
    else -> null
}
