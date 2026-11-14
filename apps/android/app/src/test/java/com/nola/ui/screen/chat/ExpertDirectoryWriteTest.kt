package com.nola.ui.screen.chat

import com.nola.data.repository.PersistentAgent
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Test

class ExpertDirectoryWriteTest {
    @Test fun reconciliationRequiresTheNewerExactAttemptedDefinition() {
        val attempted = PersistentAgent("stable-id", "Writer", 3, "active", buildJsonObject { put("expertReasoningEffort", "high") }, "Implement")
        assertFalse(confirmsExpertDefinition(attempted, attempted))
        assertTrue(confirmsExpertDefinition(attempted.copy(revision = 4), attempted))
        assertFalse(confirmsExpertDefinition(attempted.copy(revision = 4, name = "Other"), attempted))
        assertFalse(confirmsExpertDefinition(attempted.copy(revision = 4, defaults = buildJsonObject {}), attempted))
        assertFalse(confirmsExpertDefinition(attempted.copy(revision = 4, availability = "archived"), attempted))
    }
}
