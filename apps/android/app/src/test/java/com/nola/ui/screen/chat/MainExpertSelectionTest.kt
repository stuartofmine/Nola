// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.RoomParticipant
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class MainExpertSelectionTest {
    @Test fun blankSelectionStaysOnlyIdsAndRevisionUntilCoreMainAdmission() {
        val value = checkNotNull(mainExpertSelection(setOf("expert"), mapOf("expert" to 2L), emptyMap()))
        assertEquals(JsonPrimitive(1), value["version"])
        val selected = value.getValue("selected_experts").jsonArray.single().jsonObject
        assertEquals(JsonPrimitive("expert"), selected["agent_id"])
        assertEquals(JsonNull, selected["participant_id"]); assertEquals(JsonNull, selected["context_conversation_id"])
        assertNull(mainExpertSelection(emptySet(), emptyMap(), emptyMap()))
    }
    @Test fun existingSelectionKeepsTheExactOldSnapshotAndRejectsMixedPins() {
        val member = RoomParticipant("member", "expert", 2, "Frozen name", "child", true, "")
        val value = checkNotNull(mainExpertSelection(setOf("expert"), mapOf("expert" to 2L), mapOf("expert" to member)))
        val selected = value.getValue("selected_experts").jsonArray.single().jsonObject
        assertEquals(JsonPrimitive("member"), selected["participant_id"]); assertEquals(JsonPrimitive("child"), selected["context_conversation_id"])
        for (bad in listOf(member.copy(agentId = "foreign"), member.copy(agentRevision = 3), member.copy(active = false))) {
            try { mainExpertSelection(setOf("expert"), mapOf("expert" to 2L), mapOf("expert" to bad)); fail("Mixed selection accepted") }
            catch (_: IllegalArgumentException) { }
        }
    }
    @Test fun capabilityGuidanceUsesOnlyFixedProtocolCodes() {
        assertTrue(mainExpertRequiresTools("main_expert_selection_requires_tool_capable_main_model"))
        assertTrue(mainExpertRequiresTools("main_expert_selection_requires_enabled_expert_dispatch"))
        assertFalse(mainExpertRequiresTools(null))
        assertFalse(mainExpertRequiresTools("please main_expert_selection_requires_tool_capable_main_model"))
        val event = org.nola.app.sse.SseEventDto(type = "submission_rejected", code = "request_rejected_before_admission", reasonCode = "main_expert_selection_requires_enabled_expert_dispatch", message = "Rejected").toDomain()
        assertTrue(event is com.nola.data.remote.sse.SseEvent.Error && mainExpertRequiresTools(event.code))
    }
}
