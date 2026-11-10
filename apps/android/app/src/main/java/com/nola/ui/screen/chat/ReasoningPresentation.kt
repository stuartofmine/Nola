package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1

/** Published text remains visible when a later tool or UI surface arrives. */
internal fun executionAnswer(parts: List<ConversationProjectionV1.Part>, fallback: String): String {
    val text = parts.filterIsInstance<ConversationProjectionV1.Part.Text>()
    // Canonical disclosure wins even when it is empty: never reveal a withheld
    // review through a stale flattened message string.
    return if (text.isEmpty()) fallback else text.joinToString("") { it.disclosedMarkdown }
}

/** Message liveness wins over stale part state when restoring history. */
internal fun reasoningIsFinished(state: ConversationProjectionV1.ActivityState, isLive: Boolean): Boolean =
    !isLive || state in setOf(
        ConversationProjectionV1.ActivityState.Completed,
        ConversationProjectionV1.ActivityState.Sealed,
        ConversationProjectionV1.ActivityState.Cancelled,
        ConversationProjectionV1.ActivityState.Failed,
    )

internal fun visibleReasoningPlaceholderId(parts: List<ConversationProjectionV1.Part>, isLive: Boolean): String? =
    if (!isLive) null else (parts.lastOrNull {
        it is ConversationProjectionV1.Part.Reasoning || it is ConversationProjectionV1.Part.Tool
    } as? ConversationProjectionV1.Part.Reasoning)
        ?.takeUnless { reasoningIsFinished(it.state, isLive) }?.partId
