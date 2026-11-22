package com.nola.data.repository

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ConversationSettingsDecodingTest {
    @Test
    fun decodesNullAgentRoomOwnerAndNullKnowledgeFieldsSafely() {
        val raw = """
            {
                "version": 1,
                "agentRoomOwner": null,
                "knowledge": {
                    "keyPointsMarkdown": null,
                    "wikiMarkdown": null
                },
                "selectedPromptTemplateID": null
            }
        """.trimIndent()

        val decoded = decodeConversationSettings(raw)

        assertNull(decoded.agentRoomOwner)
        assertEquals("", decoded.knowledgePoints)
        assertEquals("", decoded.wiki)
        assertNull(decoded.selectedPromptTemplateId)
    }

    @Test
    fun malformedSettingsJsonFallsBackToDefaultsWithoutThrowing() {
        val decoded = decodeConversationSettings("{not-valid-json")

        assertNull(decoded.agentRoomOwner)
        assertEquals("", decoded.knowledgePoints)
        assertEquals("", decoded.wiki)
    }

    @Test
    fun decodesValidAgentRoomOwnerAndKnowledgeFields() {
        val raw = """
            {
                "version": 1,
                "agentRoomOwner": {
                    "room_id": "room-1",
                    "participant_id": "part-1",
                    "agent_id": "agent-1"
                },
                "knowledge": {
                    "keyPointsMarkdown": "- key point",
                    "wikiMarkdown": "# Wiki"
                },
                "selectedPromptTemplateID": "tpl-1"
            }
        """.trimIndent()

        val decoded = decodeConversationSettings(raw)

        assertEquals("room-1", decoded.agentRoomOwner?.roomId)
        assertEquals("part-1", decoded.agentRoomOwner?.participantId)
        assertEquals("agent-1", decoded.agentRoomOwner?.agentId)
        assertEquals("- key point", decoded.knowledgePoints)
        assertEquals("# Wiki", decoded.wiki)
        assertEquals("tpl-1", decoded.selectedPromptTemplateId)
    }
}
