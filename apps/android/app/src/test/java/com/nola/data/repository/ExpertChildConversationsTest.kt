// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class ExpertChildConversationsTest {
    private val child = ExpertChildConversation("child", 2, "Frozen expert", "member", "agent", 1, true, true, 1, 2, 3)
    private val page = ExpertChildConversationPage(version = 1, conversationId = "parent", roomId = "room", roomRevision = 1, children = listOf(child), hasMore = false, nextOffset = null)

    @Test fun historicalRevisionStaysUnknownAndPageMustBelongToParent() {
        assertNull(page.copy(children = listOf(child.copy(agentRevision = null, isCurrent = false))).validated("parent", 0).children.single().agentRevision)
        for (invalid in listOf(page.copy(version = 2), page.copy(conversationId = "other"), page.copy(children = listOf(child, child)),
            page.copy(children = listOf(child.copy(conversationId = "parent"))), page.copy(hasMore = true, nextOffset = 0), page.copy(nextOffset = 1))) {
            assertTrue(runCatching { invalid.validated("parent", 0) }.isFailure)
        }
        assertEquals(41, page.copy(hasMore = true, nextOffset = 41).validated("parent", 40).nextOffset)
    }

    @Test fun continuePinsExactMemberWithoutLookingUpLatestDirectory() {
        val member = RoomParticipant("member", "agent", 1, "Frozen", "child", true, "")
        val room = AgentRoomSnapshot(1, AgentRoom("room", "", listOf(member)))
        assertEquals(member, currentExpertChildMember(room, child))
        for (historical in listOf(child.copy(agentRevision = null), child.copy(isCurrent = false), child.copy(active = false), child.copy(participantId = "new"), child.copy(conversationId = "new-child"))) {
            assertNull(currentExpertChildMember(room, historical))
        }
    }

    @Test fun explicitPeerIdentityAndFrozenDisplayNameOwnSenderLabel() {
        val peer = """{"exchange_id":"exchange","parent_conversation_id":"parent","target":{"context_conversation_id":"child","participant_id":"member","agent_id":"agent"},"source":{"agent_id":"source"},"source_display_name":"审查专家"}"""
        assertEquals(mapOf("exchange:user" to ExpertHistorySender("审查专家", "source")), expertChildExchangeSenders(Json.parseToJsonElement("[$peer]"), "parent", child))
        assertTrue(expertChildExchangeSenders(null, "parent", child).isEmpty())
        assertTrue(runCatching { expertChildExchangeSenders(Json.parseToJsonElement("[$peer,$peer]"), "parent", child) }.isFailure)
        assertTrue(runCatching { expertChildExchangeSenders(Json.parseToJsonElement("[$peer]"), "other", child) }.isFailure)
    }

    @Test fun mainSourceMustBeTheActualParentAndUnknownKindsDoNotBecomeLegacyExperts() {
        val peer = """{"exchange_id":"main-exchange","parent_conversation_id":"parent","target":{"context_conversation_id":"child","participant_id":"member","agent_id":"agent"},"source":{"kind":"main_agent","context_conversation_id":"parent"}}"""
        assertEquals(mapOf("main-exchange:user" to ExpertHistorySender("Nola", "parent")), expertChildExchangeSenders(Json.parseToJsonElement("[$peer]"), "parent", child))
        assertTrue(runCatching { expertChildExchangeSenders(Json.parseToJsonElement("[${peer.replace("context_conversation_id\":\"parent", "context_conversation_id\":\"foreign")}]"), "parent", child) }.isFailure)
        assertTrue(runCatching { expertChildExchangeSenders(Json.parseToJsonElement("[${peer.replace("main_agent", "unknown")}]"), "parent", child) }.isFailure)
    }
}
