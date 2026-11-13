package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1

/** Renderer-only grouping. Input order is the canonical order, never a sort key. */
internal sealed interface CanonicalMessageBlock {
    val key: String
    data class Activity(val parts: List<ConversationProjectionV1.Part>) : CanonicalMessageBlock {
        override val key: String get() = "execution:${parts.first().partId}"
    }
    data class Part(val part: ConversationProjectionV1.Part) : CanonicalMessageBlock {
        override val key: String get() = part.partId
    }
}

internal fun isExecutionActivity(part: ConversationProjectionV1.Part): Boolean =
    part is ConversationProjectionV1.Part.Reasoning || part is ConversationProjectionV1.Part.Tool ||
        (part is ConversationProjectionV1.Part.Historical && part.projection.canonicalKind in setOf("reasoning", "tool"))

/** One message-owned disclosure at the first execution anchor; other anchors keep their order. */
internal fun canonicalMessageBlocks(parts: List<ConversationProjectionV1.Part>): List<CanonicalMessageBlock> = buildList {
    val activity = parts.filter(::isExecutionActivity)
    var inserted = false
    parts.forEach { part ->
        if (isExecutionActivity(part)) {
            if (!inserted) { add(CanonicalMessageBlock.Activity(activity)); inserted = true }
            // Historical reader actions remain reachable even if reasoning is hidden/empty.
            if (part is ConversationProjectionV1.Part.Historical) add(CanonicalMessageBlock.Part(part))
        } else add(CanonicalMessageBlock.Part(part))
    }
}

internal fun visibleExecutionActivities(parts: List<ConversationProjectionV1.Part>, isLive: Boolean, thinkingEnabled: Boolean): List<ConversationProjectionV1.Part> {
    val placeholderId = visibleReasoningPlaceholderId(parts, isLive)
    return parts.filter { part -> when (part) {
        is ConversationProjectionV1.Part.Reasoning -> thinkingEnabled && (!part.summary.isNullOrBlank() || part.partId == placeholderId)
        is ConversationProjectionV1.Part.Historical -> part.projection.canonicalKind != "reasoning" ||
            (thinkingEnabled && !part.projection.decodedText.isNullOrBlank())
        else -> true
    } }
}

internal fun hasLegacyExecutionDisclosure(hasCanonicalActivities: Boolean, thinkingEnabled: Boolean, thinking: String,
    hasTools: Boolean, waitingForFirstContent: Boolean): Boolean = !hasCanonicalActivities &&
        ((thinkingEnabled && thinking.isNotBlank()) || hasTools || waitingForFirstContent)

/** Ephemeral disclosure choices belong to a message owner, never a list position. */
internal data class CanonicalActivityDisclosureChoices(
    val detailsPartIds: Set<String> = emptySet(),
    val expandedByPart: Map<String, Boolean> = emptyMap(),
) {
    fun expandedFor(parts: List<ConversationProjectionV1.Part>): Boolean? {
        val choices = parts.mapNotNull { expandedByPart[it.partId] }
        return when { false in choices -> false; true in choices -> true; else -> null }
    }

    fun expanded(parts: List<ConversationProjectionV1.Part>, value: Boolean) =
        copy(expandedByPart = expandedByPart + parts.associate { it.partId to value })

    fun details(parts: List<ConversationProjectionV1.Part>, partId: String, value: Boolean): CanonicalActivityDisclosureChoices {
        val next = copy(detailsPartIds = if (value) detailsPartIds + partId else detailsPartIds - partId)
        return if (value) next.expanded(parts, true) else next
    }
}
