// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.nola.data.repository.RoomParticipant
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

/** UTF-16 ranges stay within Compose. The wire carries ordered text segments. */
@Serializable
data class InlineExpertMention(
    val start: Int, val label: String, val agentId: String, val revision: Long,
    val member: RoomParticipant? = null, val host: String? = null, val epoch: String? = null,
) {
    val end get() = start + label.length
}

internal data class ExpertQuery(val start: Int, val end: Int, val text: String)

internal fun expertQuery(value: TextFieldValue, mentions: List<InlineExpertMention>): ExpertQuery? {
    if (!value.selection.collapsed || value.composition != null) return null
    val end = value.selection.end
    val start = value.text.lastIndexOf('@', startIndex = end - 1)
    if (start < 0 || mentions.any { start in it.start until it.end }) return null
    val previous = value.text.getOrNull(start - 1)
    if (previous != null && (previous in 'A'..'Z' || previous in 'a'..'z' || previous in '0'..'9' || previous in "._%+-")) return null
    val query = value.text.substring(start + 1, end)
    return if (query.all { it.isLetterOrDigit() || it == '_' || it == '-' }) ExpertQuery(start, end, query) else null
}

internal data class MentionEdit(val value: TextFieldValue, val mentions: List<InlineExpertMention>)

/** Preserve only spans known to be outside this edit, including with equal names on both sides. */
internal fun editExpertText(old: TextFieldValue, next: TextFieldValue, mentions: List<InlineExpertMention>): MentionEdit {
    if (old.text == next.text || mentions.isEmpty()) return MentionEdit(next, mentions)
    val before = old.text
    val after = next.text
    val prefix = before.commonPrefixWith(after).length.coerceAtMost(minOf(old.selection.min, next.selection.min))
    val suffix = before.substring(prefix).commonSuffixWith(after.substring(prefix)).length
        .coerceAtMost(minOf(before.length - old.selection.max, after.length - next.selection.max))
    val oldEnd = before.length - suffix
    val newEnd = after.length - suffix
    if (old.selection.collapsed && before.length - after.length == 1 && newEnd == prefix) {
        val hit = mentions.find { prefix >= it.start && oldEnd <= it.end && (old.selection.end == it.end || old.selection.start == it.start) }
        if (hit != null) return MentionEdit(TextFieldValue(before.removeRange(hit.start, hit.end), TextRange(hit.start)),
            mentions.filter { it != hit }.map { if (it.start >= hit.end) it.copy(start = it.start - hit.label.length) else it })
    }
    return MentionEdit(next, mentions.mapNotNull {
        when {
            it.end <= prefix -> it
            it.start >= oldEnd -> it.copy(start = it.start + after.length - before.length)
            else -> null
        }
    })
}

internal fun insertExpert(value: TextFieldValue, mentions: List<InlineExpertMention>, range: TextRange, ref: InlineExpertMention): MentionEdit {
    val start = range.min
    val end = range.max
    val delta = ref.label.length - (end - start)
    val kept = mentions.mapNotNull {
        when {
            it.end <= start -> it
            it.start >= end -> it.copy(start = it.start + delta)
            else -> null
        }
    }
    return MentionEdit(TextFieldValue(value.text.replaceRange(start, end, ref.label), TextRange(start + ref.label.length)),
        (kept + ref.copy(start = start)).sortedBy { it.start })
}

internal fun mentionSegments(text: String, mentions: List<InlineExpertMention>): JsonObject = buildJsonObject {
    put("version", 1)
    put("segments", buildJsonArray {
        var offset = 0
        for (ref in mentions) {
            require(ref.start >= offset && ref.label.startsWith('@') && ref.label.length > 1 && text.startsWith(ref.label, ref.start))
            if (ref.start > offset) add(buildJsonObject { put("text", text.substring(offset, ref.start)) })
            add(buildJsonObject { put("agent_id", ref.agentId); put("text", ref.label) })
            offset = ref.end
        }
        if (offset < text.length) add(buildJsonObject { put("text", text.substring(offset)) })
    })
}

internal fun mentionOnly(text: String, mentions: List<InlineExpertMention>): Boolean {
    if (mentions.isEmpty()) return false
    var offset = 0
    for (mention in mentions) {
        if (text.substring(offset, mention.start).isNotBlank()) return false
        offset = mention.end
    }
    return text.substring(offset).isBlank()
}

internal fun inlineSelection(text: String, mentions: List<InlineExpertMention>): JsonObject? {
    if (mentions.isEmpty()) return null
    val refs = mentions.distinctBy { it.agentId }
    require(mentions.all { ref -> refs.first { it.agentId == ref.agentId }.let { it.revision == ref.revision && it.member == ref.member } })
    require(refs.all { it.host == refs.first().host && it.epoch == refs.first().epoch })
    val ids = refs.map { it.agentId }.toSet()
    val selected = checkNotNull(mainExpertSelection(ids, refs.associate { it.agentId to it.revision },
        refs.mapNotNull { ref -> ref.member?.let { ref.agentId to it } }.toMap()))
    return buildJsonObject {
        selected.forEach { (k,v) -> put(k,v) }
        put("expert_mentions", mentionSegments(text, mentions))
        refs.first().epoch?.let { put("expected_source_store_epoch", it) }
    }
}

internal fun historyMentions(text: String, payload: JsonObject?): List<InlineExpertMention> {
    if (payload == null) return emptyList()
    require(payload.getValue("version").jsonPrimitive.int == 1)
    val refs = payload.getValue("selected_experts").jsonArray.map { it.jsonObject }
        .associateBy { it.getValue("agent_id").jsonPrimitive.content }
    var offset = 0
    val mentions = mutableListOf<InlineExpertMention>()
    for (segment in payload.getValue("segments").jsonArray) {
        val part = segment.jsonObject
        val value = part.getValue("text").jsonPrimitive.content
        require(text.startsWith(value, offset))
        part["agent_id"]?.jsonPrimitive?.contentOrNull?.let { id ->
            val ref = refs.getValue(id)
            val revision = ref.getValue("agent_revision").jsonPrimitive.long
            val participant = ref.getValue("participant_id").jsonPrimitive.content
            val child = ref.getValue("context_conversation_id").jsonPrimitive.content
            require(value.startsWith('@') && value.length > 1 && participant.isNotBlank() && child.isNotBlank())
            mentions += InlineExpertMention(offset, value, id, revision,
                RoomParticipant(participant, id, revision, value.drop(1), child, true, ""))
        }
        offset += value.length
    }
    require(offset == text.length && refs.keys == mentions.map { it.agentId }.toSet())
    return mentions
}

internal fun com.nola.domain.model.ConversationMessage.expertMentionHistory(): JsonObject? =
    historicalParts.mapNotNull { part ->
        part.projection.takeIf { it.canonicalKind == "text" }?.expertMentions
    }.singleOrNull()

internal fun InlineExpertMention.matchesHistory(child: com.nola.data.repository.ExpertChildConversation): Boolean {
    val pinned = member ?: return false
    return child.conversationId == pinned.contextConversationId && child.participantId == pinned.id &&
        child.agentId == agentId && (child.agentRevision == revision || (!child.isCurrent && child.agentRevision == null))
}

/** Equal names and roles still need a visible distinction in the picker. */
internal fun mentionDescriptions(agents: List<com.nola.data.repository.PersistentAgent>): Map<String, String> =
    agents.groupBy { it.name to it.description }.values.flatMap { group ->
        if (group.size == 1) listOf(group.first().let { it.id to it.description })
        else {
            val length = (4..group.maxOf { it.id.length }).firstOrNull { size -> group.map { it.id.takeLast(size) }.distinct().size == group.size }
            group.map { agent -> agent.id to listOf(agent.description, "#" + agent.id.takeLast(length ?: agent.id.length)).filter(String::isNotBlank).joinToString(" · ") }
        }
    }.toMap()
