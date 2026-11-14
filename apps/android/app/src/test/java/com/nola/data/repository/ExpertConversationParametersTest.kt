// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import com.nola.domain.model.ModelInfo
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertConversationParametersTest {
    private val model = ModelInfo("target-model", "Target", profileId = "profile", profileRevision = 2)
    @Test fun sparseExactExpertEffortPreservesOwnProfileLimitsAndPresentation() {
        val settings = Json.parseToJsonElement("""{"modelOverrides":{"configured:profile":{"reasoningPreference":"high"}},"clientModelPresentation":{"configured:profile":{"customSystemPrompt":"Target instructions","showThinking":false}}}""").jsonObject
        val declared = Json.parseToJsonElement("""{"maxContextLength":32768,"maxOutputTokens":1024,"temperature":0.2,"topP":0.7,"reasoningPreference":"low"}""").jsonObject
        val actual = readExpertConversationParameters(settings, model, declared)
        assertEquals("high", actual.reasoningPreference)
        assertEquals(1024, actual.maxCompletionTokens); assertEquals(32768, actual.maxContextTokens)
        assertEquals(0.2f, actual.temperature); assertEquals(0.7f, actual.topP)
        assertEquals("Target instructions", actual.customSystemPrompt); assertFalse(actual.enableThinking)
    }
    @Test fun undeclaredEffortRemainsInheritedAndForeignProfileOverrideIsIgnored() {
        val settings = Json.parseToJsonElement("""{"modelOverrides":{"configured:foreign":{"reasoningPreference":"none"}}}""").jsonObject
        val actual = readExpertConversationParameters(settings, model, buildJsonObject { put("maxOutputTokens", 4096) })
        assertNull(actual.reasoningPreference); assertEquals(4096, actual.maxCompletionTokens)
        val inherited=readExpertConversationParameters(settings,model,buildJsonObject {put("customSystemPrompt","Profile instruction");put("showThinking",false)})
        assertEquals("Profile instruction",inherited.customSystemPrompt);assertFalse(inherited.enableThinking)
    }
}
