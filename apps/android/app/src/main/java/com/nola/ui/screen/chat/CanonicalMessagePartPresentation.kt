package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.sourceCardFacts

/** Renderer-only grouping. Input order is the canonical order, never a sort key. */
internal sealed interface CanonicalMessageBlock {
    val key: String
    data class Activity(val parts: List<ConversationProjectionV1.Part>) : CanonicalMessageBlock {
        override val key: String get() = parts.first().partId
    }
    data class Part(val part: ConversationProjectionV1.Part) : CanonicalMessageBlock {
        override val key: String get() = part.partId
    }
}

internal fun canonicalMessageBlocks(parts: List<ConversationProjectionV1.Part>): List<CanonicalMessageBlock> = buildList {
    val activity = mutableListOf<ConversationProjectionV1.Part>()
    fun flush() { if (activity.isNotEmpty()) { add(CanonicalMessageBlock.Activity(activity.toList())); activity.clear() } }
    parts.forEach { part ->
        if (part is ConversationProjectionV1.Part.Reasoning || part is ConversationProjectionV1.Part.Tool) activity += part
        else { flush(); add(CanonicalMessageBlock.Part(part)) }
    }
    flush()
}

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

/** Only display aliases collapse. Canonical Part ownership and pinpoint ranges remain intact. */
internal fun canonicalBibliography(parts: List<ConversationProjectionV1.Part>) = parts.mapNotNull { part ->
    part.sourceCardFacts()?.let { part to it }
}.distinctBy { (_, facts) -> listOf(facts.uri.ifBlank { facts.sourceId }, facts.location, facts.range) }

internal fun executionProcessHeading(live: Boolean, currentTitle: String?, processing: String, history: String): String =
    if (live) currentTitle?.takeIf { it.isNotBlank() } ?: processing else history
