package com.nola.domain.projection

import com.nola.domain.model.AnswerSource
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.encodeAgentTraceProjectionForMeta
import com.nola.domain.model.encodeAnswerSourcesForMeta
import com.nola.domain.model.encodeHostToolEventsForMeta

internal const val CONVERSATION_SURFACE_LIVE_META_KEY = "nola.surface.live"
internal const val CONVERSATION_SURFACE_PLACEHOLDER_META_KEY = "nola.surface.placeholder"
internal const val CONVERSATION_SURFACE_CURRENT_TOOL_CALL_META_KEY = "nola.surface.currentToolCallID"
internal const val CONVERSATION_SURFACE_MESSAGE_STATE_META_KEY = "nola.surface.messageState"

/**
 * Projects the canonical Application read model into Android's chat history model.
 *
 * This type is intentionally pure. It never repairs Host events, writes Store
 * rows, or combines transport deltas. The Application projection is the only
 * durable source; Compose may layer an active response over these messages
 * while a Run is in progress.
 */
object CanonicalConversationProjector {
    /** Merge one committed message without dropping paged history or replacing
     * unrelated rows with an older bounded admission window. */
    fun mergeJournalMessage(
        existing: List<ConversationMessage>,
        admissionWindow: List<ConversationMessage>,
        updated: ConversationMessage,
    ): List<ConversationMessage> {
        require(existing.all { it.conversationId == updated.conversationId } &&
            admissionWindow.all { it.conversationId == updated.conversationId })
        require(admissionWindow.any { it.id == updated.id }) { "Journal message has no canonical admission" }
        val knownIds = existing.mapTo(HashSet()) { it.id }
        if (admissionWindow.all { it.id in knownIds }) {
            return existing.map { if (it.id == updated.id) updated else it }
        }
        val messages = existing.toMutableList()
        for ((position, admitted) in admissionWindow.withIndex()) {
            if (messages.any { it.id == admitted.id }) continue
            val preceding = admissionWindow.take(position).asReversed()
                .firstNotNullOfOrNull { prior -> messages.indexOfFirst { it.id == prior.id }.takeIf { it >= 0 } }
            val following = admissionWindow.drop(position + 1)
                .firstNotNullOfOrNull { next -> messages.indexOfFirst { it.id == next.id }.takeIf { it >= 0 } }
            messages.add(preceding?.plus(1) ?: following ?: messages.size, admitted)
        }
        val index = messages.indexOfFirst { it.id == updated.id }
        check(index >= 0)
        messages[index] = updated
        return messages
    }

    fun project(projection: ConversationProjectionV1): List<ConversationMessage> =
        orderedMessages(projection)
            // System messages remain in the canonical projection for audit and
            // replay, but they are protocol context rather than chat timeline
            // rows. iOS applies the same presentation boundary.
            .filter { it.role != ConversationProjectionV1.Role.System }
            .map { message ->
            val text = message.parts
                .mapNotNull { part -> when (part) {
                    is ConversationProjectionV1.Part.Text -> part.disclosedMarkdown
                    is ConversationProjectionV1.Part.UiAction -> part.summary
                    is ConversationProjectionV1.Part.Historical -> part.projection.let {
                        if (it.canonicalKind == "reasoning" && it.decodedText != null) null
                        else it.timelineText
                    }
                    else -> null
                } }.joinToString(separator = "")
            val reasoning = message.parts
                .mapNotNull { part -> when (part) {
                    is ConversationProjectionV1.Part.Reasoning -> part.summary
                    is ConversationProjectionV1.Part.Historical -> if (part.projection.canonicalKind == "reasoning") part.projection.decodedText else null
                    else -> null
                } }
                .joinToString(separator = "\n")
            val tools = message.parts
                .filterIsInstance<ConversationProjectionV1.Part.Tool>()
                .map(::toolEvent)
            val sources = message.parts.mapNotNull(::answerSource)
            val currentToolCallId = message.parts
                .filterIsInstance<ConversationProjectionV1.Part.Tool>()
                .firstOrNull { it.partId == projection.surface.currentActivityPartId }
                ?.toolCallId
            val metadata = buildMap {
                encodeHostToolEventsForMeta(tools)?.let(::putAll)
                putAll(encodeAnswerSourcesForMeta(sources))
                putAll(
                    encodeAgentTraceProjectionForMeta(
                        thinking = reasoning,
                        phase = message.state.name.lowercase(),
                    ),
                )
                put("nola.canonicalMessageID", message.messageId)
                put("nola.canonicalMessageRevision", message.revision.toString())
                if (message.role == ConversationProjectionV1.Role.Assistant) {
                    put("nola.context.usedTokens", projection.contextUsage.usedTokens.toString())
                    put("nola.context.limitTokens", projection.contextUsage.windowTokens.toString())
                    put(
                        CONVERSATION_SURFACE_LIVE_META_KEY,
                        (projection.surface.liveMessageId == message.messageId).toString(),
                    )
                    put(
                        CONVERSATION_SURFACE_PLACEHOLDER_META_KEY,
                        (
                            projection.surface.liveMessageId == message.messageId &&
                                projection.surface.showAssistantPlaceholder
                            ).toString(),
                    )
                    put(CONVERSATION_SURFACE_MESSAGE_STATE_META_KEY, message.state.name.lowercase())
                    currentToolCallId?.let {
                        put(CONVERSATION_SURFACE_CURRENT_TOOL_CALL_META_KEY, it)
                    }
                }
            }
            ConversationMessage(
                id = message.messageId,
                conversationId = projection.conversationId,
                role = when (message.role) {
                    ConversationProjectionV1.Role.User -> ChatRole.user
                    ConversationProjectionV1.Role.Assistant -> ChatRole.assistant
                    ConversationProjectionV1.Role.System -> ChatRole.system
                },
                text = text,
                createdAt = message.createdAtMs,
                meta = metadata.takeIf(Map<String, String>::isNotEmpty),
                historicalParts = message.parts.filterIsInstance<ConversationProjectionV1.Part.Historical>(),
                canonicalParts = message.parts,
            )
            }

    private fun orderedMessages(
        projection: ConversationProjectionV1,
    ): List<ConversationProjectionV1.Message> {
        val byId = projection.messages.associateBy { it.messageId }
        val orderedIds = buildList {
            addAll(projection.surface.committedMessageIds)
            projection.surface.liveMessageId?.let(::add)
            addAll(projection.messages.map { it.messageId })
        }.distinct()
        return orderedIds.mapNotNull(byId::get)
    }

    private fun toolEvent(part: ConversationProjectionV1.Part.Tool) = HostAgentToolEvent(
        id = part.toolCallId,
        toolName = part.capabilityId,
        argsPreview = part.input.toString(),
        status = when (part.state) {
            ConversationProjectionV1.ActivityState.Pending -> "pending"
            ConversationProjectionV1.ActivityState.Running -> "running"
            ConversationProjectionV1.ActivityState.WaitingForPermission -> "permission_required"
            ConversationProjectionV1.ActivityState.WaitingForInput -> "waiting_for_input"
            ConversationProjectionV1.ActivityState.Completed -> "completed"
            ConversationProjectionV1.ActivityState.Sealed -> "sealed"
            ConversationProjectionV1.ActivityState.Cancelled -> "cancelled"
            ConversationProjectionV1.ActivityState.Failed -> "failed"
        },
        resultSummary = part.summary,
        detailPreview = part.outputPreview,
        error = part.state == ConversationProjectionV1.ActivityState.Failed,
    )

    private fun answerSource(part: ConversationProjectionV1.Part): AnswerSource? = when (part) {
        is ConversationProjectionV1.Part.Source -> AnswerSource(
            id = part.partId,
            sourceId = part.sourceId,
            title = part.title,
            uri = part.uri,
            mimeType = part.mimeType,
        )
        is ConversationProjectionV1.Part.Citation -> AnswerSource(
            id = part.citationId,
            sourceId = part.sourceId,
            title = part.title,
            uri = part.uri,
            mimeType = part.mimeType,
            page = part.page?.asLegacyLongOrNull(),
            lineStart = part.lineStart?.asLegacyLongOrNull(),
            lineEnd = part.lineEnd?.asLegacyLongOrNull(),
            section = part.section,
        )
        else -> null
    }
}

object CompactionSourceNavigator {
    data class Request(
        val conversationId: String,
        val compactionPartId: String,
        val sourceItemIds: List<String>,
        val sourceFingerprint: String,
    )

    fun accepts(
        request: Request,
        currentConversationId: String?,
        messages: List<ConversationMessage>,
    ): Boolean = request.conversationId == currentConversationId &&
        request.sourceItemIds.isNotEmpty() &&
        request.sourceFingerprint.isNotBlank() &&
        messages.any { message ->
            message.canonicalParts.any { part ->
                part is ConversationProjectionV1.Part.Compaction &&
                    part.partId == request.compactionPartId &&
                    part.sourceItemIds == request.sourceItemIds &&
                    part.sourceFingerprint == request.sourceFingerprint
            }
        }

    fun accepts(
        request: Request,
        currentConversationId: String?,
        projection: ConversationProjectionV1,
    ): Boolean = request.conversationId == currentConversationId &&
        request.sourceItemIds.isNotEmpty() &&
        request.sourceFingerprint.isNotBlank() &&
        projection.conversationId == currentConversationId &&
        projection.messages.any { message ->
            message.parts.any { part ->
                part is ConversationProjectionV1.Part.Compaction &&
                    part.partId == request.compactionPartId &&
                    part.sourceItemIds == request.sourceItemIds &&
                    part.sourceFingerprint == request.sourceFingerprint
            }
        }

    fun belongsToSnapshot(
        window: ConversationProjectionWindowV1,
        conversationId: String,
        revision: Long,
        eventSequence: Long,
    ): Boolean = window.projection.conversationId == conversationId &&
        window.projection.revision == revision &&
        window.eventSequence == eventSequence

    fun isCurrentLookup(
        requestedConversationId: String,
        currentConversationId: String?,
        requestedLoadGeneration: Long,
        currentLoadGeneration: Long,
    ): Boolean = requestedConversationId == currentConversationId &&
        requestedLoadGeneration == currentLoadGeneration

    fun targetMessageId(sourceItemIds: List<String>, messages: List<ConversationMessage>): String? {
        val sources = sourceItemIds.toSet()
        return messages.firstOrNull { message ->
            message.meta?.get("nola.canonicalMessageID") in sources ||
                message.canonicalParts.any { it.partId in sources }
        }?.id
    }

    fun mergeOlder(older: List<ConversationMessage>, loaded: List<ConversationMessage>): List<ConversationMessage> =
        (older + loaded).distinctBy { it.meta?.get("nola.canonicalMessageID") ?: it.id }

    fun mergeFetchedOlderPages(
        pagesNewestToOldest: List<List<ConversationMessage>>,
        current: List<ConversationMessage>,
    ): List<ConversationMessage> {
        val seen = mutableSetOf<String>()
        return buildList {
            pagesNewestToOldest.asReversed().forEach { page ->
                page.forEach { message ->
                    val identity = message.meta?.get("nola.canonicalMessageID") ?: message.id
                    if (seen.add(identity)) add(message)
                }
            }
            current.forEach { message ->
                val identity = message.meta?.get("nola.canonicalMessageID") ?: message.id
                if (seen.add(identity)) add(message)
            }
        }
    }

    fun canContinue(previousCursor: Long, nextCursor: Long?, hasMore: Boolean): Boolean =
        hasMore && nextCursor != null && nextCursor < previousCursor
}
