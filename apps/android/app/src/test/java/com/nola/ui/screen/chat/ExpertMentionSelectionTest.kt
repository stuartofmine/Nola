package com.nola.ui.screen.chat

import com.nola.data.repository.*
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.*
import org.junit.Test

class ExpertMentionSelectionTest {
    private val agent = PersistentAgent("opaque-agent", "Search current / 编写", 2, "active", buildJsonObject {}, "")
    private val old = RoomParticipant("old", agent.id, 1, "Old name", "old-context", true, "")
    private val before = AgentRoomSnapshot(4, AgentRoom("room", "", listOf(old)))
    private val fresh = old.copy(id = "new", agentRevision = 2, contextConversationId = "fresh-context")
    private val after = AgentRoomSnapshot(5, AgentRoom("room", "", listOf(fresh)))

    @Test fun candidateUsesOpaqueIdentityAndRevisionRatherThanTextAndIncludesNonMembers() {
        val directory = AgentDirectory(3, listOf(agent))
        assertEquals(agent, checkedExpertCandidate(directory, agent.id, 2))
        assertNull(currentExpertParticipant(before.copy(room = before.room.copy(participants = emptyList())), agent))
        assertThrows(ExpertSelectionChanged::class.java) { checkedExpertCandidate(directory, agent.name, 2) }
        assertThrows(ExpertSelectionChanged::class.java) { checkedExpertCandidate(directory, agent.id, 1) }
        assertThrows(ExpertSelectionChanged::class.java) { checkedExpertCandidate(directory.copy(agents = listOf(agent.copy(availability = "archived"))), agent.id, 2) }
        assertThrows(ExpertSelectionChanged::class.java) { currentExpertParticipant(before, agent) }
    }

    @Test fun explicitRejoinMustKeepHistoricalContextAndProveFreshOwnership() {
        assertEquals(fresh, rejoinedExpertParticipant(before, after, agent))
        for (malformed in listOf(before, after.copy(room = after.room.copy(id = "other")),
            after.copy(room = after.room.copy(participants = emptyList())),
            after.copy(room = after.room.copy(participants = listOf(old, fresh))),
            after.copy(room = after.room.copy(participants = listOf(old.copy(active = false), fresh.copy(contextConversationId = "old-context")))))) {
            assertThrows(ExpertSelectionChanged::class.java) { rejoinedExpertParticipant(before, malformed, agent) }
        }
    }
    @Test fun expertOnlySubmissionDoesNotRequireAnUnrelatedPrimaryModel() {
        assertTrue(requiresPrimaryModelForComposer(false, emptySet()))
        assertFalse(requiresPrimaryModelForComposer(false, setOf(agent.id)))
        assertFalse(requiresPrimaryModelForComposer(true, emptySet()))
        assertFalse(requiresPrimaryModelForComposer(true, setOf(agent.id)))
    }
    @Test fun joinedOldSnapshotContinuesWithoutFreshDirectoryButNeverChangesItsContextSilently() {
        // Directory agent is revision 2; the already-joined revision 1 remains authoritative.
        assertEquals(old, selectedExpertParticipant(before, agent.id, 1, old))
        assertThrows(ExpertSelectionChanged::class.java) { currentExpertParticipant(before, agent) }
        assertThrows(ExpertSelectionChanged::class.java) { selectedExpertParticipant(after, agent.id, 1, old) }
        assertThrows(ExpertSelectionChanged::class.java) { selectedExpertParticipant(before.copy(room = before.room.copy(participants = emptyList())), agent.id, 1, old) }
        assertNull(selectedExpertParticipant(before.copy(room = before.room.copy(participants = emptyList())), agent.id, 2, null))
    }
    @Test fun selectedChipUsesPinnedNameWithoutDirectoryAndNeverDisappearsDuringLoading() {
        assertEquals("Old name", selectedExpertLabel(agent.id, old, null, "Expert"))
        assertEquals("Old name", selectedExpertLabel(agent.id, old, AgentDirectory(3, listOf(agent)), "Expert"))
        assertEquals(agent.name, selectedExpertLabel(agent.id, null, AgentDirectory(3, listOf(agent)), "Expert"))
        assertEquals("Expert", selectedExpertLabel(agent.id, null, null, "Expert"))
        assertEquals("Expert", selectedExpertLabel("other", old, null, "Expert"))
    }

}
