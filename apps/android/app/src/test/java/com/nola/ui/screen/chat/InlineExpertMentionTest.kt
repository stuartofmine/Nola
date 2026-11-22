package com.nola.ui.screen.chat

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.projection.ConversationProjectionV1
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class InlineExpertMentionTest {
    private val ref = InlineExpertMention(0, "@专家", "a", 2)
    private fun value(text: String, caret: Int) = TextFieldValue(text, TextRange(caret))

    @Test fun cursorImeAndEmail() {
        assertEquals("研", expertQuery(value("😀请@研确认", 5), emptyList())?.text)
        val multiword = "@Arithmetic reviewer"
        assertEquals("Arithmetic reviewer", expertQuery(value(multiword, multiword.length), emptyList())?.text)
        assertNull(expertQuery(value("@ Arithmetic", 12), emptyList()))
        assertNull(expertQuery(value("@one\ntwo", 8), emptyList()))
        assertNull(expertQuery(value("someone@abc.cn", 11), emptyList()))
        assertNull(expertQuery(TextFieldValue("@研", TextRange(2), TextRange(1,2)), emptyList()))
    }

    @Test fun repeatedNamesDoNotMergePositionsOrInventIdentity() {
        val first = insertExpert(value("请@研检查", 3), emptyList(), TextRange(1,3), ref)
        val both = insertExpert(first.value, first.mentions, TextRange(4), ref)
        val selection = inlineSelection(both.value.text, both.mentions)!!
        assertEquals(1, selection.getValue("selected_experts").jsonArray.size)
        assertEquals(2, selection.getValue("expert_mentions").jsonObject.getValue("segments").jsonArray.count { it.jsonObject["agent_id"] != null })
        val deleted = editExpertText(both.value, value("请@专家@专检查", 6), both.mentions)
        assertEquals(first.value.text, deleted.value.text)
        assertEquals(first.mentions, deleted.mentions)
        val pasted = editExpertText(first.value, value("请@专家@专家检查", 7), first.mentions)
        assertEquals(first.mentions, pasted.mentions)
    }

    @Test fun frozenMentionsRestoreFromHistoryAndRejectMismatchedText() {
        val selected = inlineSelection("请@专家", listOf(ref.copy(start = 1)))!!
        val frozen = buildJsonObject {
            put("selected_experts", buildJsonArray { add(buildJsonObject {
                put("agent_id","a"); put("agent_revision",2); put("participant_id","p"); put("context_conversation_id","child")
            }) })
            selected.getValue("expert_mentions").jsonObject.forEach { (k,v) -> put(k,v) }
        }
        val pinned = historyMentions("请@专家", frozen).single()
        assertEquals("p", pinned.member?.id)
        assertTrue(runCatching { historyMentions("请@别名", frozen) }.isFailure)
        val projection = ConversationProjectionV1.HistoricalProjection(
            owner = buildJsonObject {}, canonicalRevision = 1, source = null, schema = null,
            canonicalKind = "text", binding = null, status = ConversationProjectionV1.HistoricalStatus.Supported,
            unsupportedCode = null, fields = mapOf("expert_mentions" to ConversationProjectionV1.HistoricalField.Complete(frozen)),
        )
        val parts = listOf(ConversationProjectionV1.Part.Historical("body", 1, projection),
            ConversationProjectionV1.Part.Historical("file", 1, projection.copy(canonicalKind = "attachment", fields = emptyMap())))
        val message = ConversationMessage(conversationId = "c", role = ChatRole.user, text = "请@专家", historicalParts = parts, canonicalParts = parts)
        assertEquals(frozen, message.expertMentionHistory())
        val retired = com.nola.data.repository.ExpertChildConversation("child", 3, "archived", "p", "a", null, false, false, 1, 2, 4)
        assertTrue(pinned.matchesHistory(retired))
        assertFalse(pinned.matchesHistory(retired.copy(isCurrent = true)))
        assertFalse(pinned.matchesHistory(retired.copy(conversationId = "replacement")))
        assertFalse(pinned.matchesHistory(retired.copy(participantId = "replacement")))
        assertFalse(pinned.matchesHistory(retired.copy(agentRevision = 3)))
    }
}
