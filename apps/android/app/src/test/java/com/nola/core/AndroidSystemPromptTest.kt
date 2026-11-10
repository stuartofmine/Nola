// SPDX-License-Identifier: Apache-2.0

package com.nola.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSystemPromptTest {
    @Test
    fun styleSelectionUsesOneRequestedSectionOnly() {
        val document = """
            [professional]
            PROFESSIONAL_STYLE
            [casual]
            CASUAL_STYLE
            [playful]
            PLAYFUL_STYLE
        """.trimIndent()
        val style = AndroidSystemPrompt.styleSection(document, "casual")
        assertTrue(style.contains("CASUAL_STYLE"))
        assertFalse(style.contains("PROFESSIONAL_STYLE"))
        assertFalse(style.contains("PLAYFUL_STYLE"))
    }

    @Test
    fun agentFragmentsKeepHostFactsAndUserPreferencesBelowCorePolicy() {
        val fragments = AndroidSystemPrompt.agentFragments(
            stylePrompt = "Use a calm, direct tone.",
            customSystemPrompt = "Keep examples short.",
        )

        assertEquals("android:host-identity", fragments.first().id)
        assertEquals(AgentPromptTrust.HostFact, fragments.first().trust)
        assertTrue(fragments.first().cacheable)
        assertTrue(fragments.drop(1).all { it.trust == AgentPromptTrust.UserInstruction })
        assertTrue(fragments.drop(1).all { !it.cacheable })
        assertEquals(AgentPromptLayer.ResponseStyle, fragments[1].layer)
        assertEquals(AgentPromptLayer.ExecutionPolicy, fragments[2].layer)
        assertTrue(fragments.all {
            it.trust == AgentPromptTrust.HostFact ||
                it.trust == AgentPromptTrust.HostInstruction ||
                it.trust == AgentPromptTrust.UserInstruction ||
                it.trust == AgentPromptTrust.Reference
        })
        assertTrue(fragments.any { it.content == "Keep examples short." })
    }
}
