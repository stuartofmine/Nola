// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.readConversationParameters
import com.nola.data.repository.writeConversationParameters
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class ReasoningEffortSelectionTest {
    @Test fun everyEffortSurvivesProfileAndConversationRoundTrips() {
        val model = ModelInfo("wire-model", "Fixture", profileId = "profile")
        for (effort in listOf("none", "low", "medium", "high", "xhigh", "automatic")) {
            val parameters = ModelParameters(temperature = .25f, reasoningPreference = effort)
            assertEquals(effort, Json.decodeFromString<ModelParameters>(Json.encodeToString(parameters)).reasoningPreference)
            val settings = writeConversationParameters("{\"unrelated\":true}", model, parameters)
            val restored = requireNotNull(readConversationParameters(settings, model))
            assertEquals(effort, restored.reasoningPreference)
            assertEquals(.25f, restored.temperature)
            assertTrue(settings.contains("\"unrelated\":true"))
        }
    }
}
