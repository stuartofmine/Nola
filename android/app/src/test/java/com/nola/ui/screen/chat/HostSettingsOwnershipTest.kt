package com.nola.ui.screen.chat

import com.nola.llm.provider.WebAgentProvider
import org.junit.Assert.*
import org.junit.Test

class HostSettingsOwnershipTest {
    private val config = WebAgentProvider.Config("https://host.invalid", "test-only", expectedHostId = "host-a-identity-1234")

    @Test fun selectionAndConnectionChangesRejectOldResultsIncludingFailures() {
        val gate = HostSettingsOwnership()
        val request = gate.begin("A", 1, config)
        assertTrue(gate.accepts(request, "A", 1, config))
        assertFalse(gate.accepts(request, "B", 1, config))
        assertFalse(gate.accepts(request, "A", 2, config))
        assertFalse(gate.accepts(request, "A", 1, null))
        for (changed in listOf(config.copy(token = "new"), config.copy(expectedHostId = "host-b"),
            config.copy(pinnedCertificateSha256 = "changed"), config.copy(hostBaseUrl = "https://other.invalid"))) {
            assertFalse(gate.accepts(request, "A", 1, changed))
        }
    }

    @Test fun laterRefreshWinsEvenForSameConversationAndConnection() {
        val gate = HostSettingsOwnership()
        val first = gate.begin(null, 1, config)
        val second = gate.begin(null, 1, config)
        assertFalse(gate.accepts(first, null, 1, config))
        assertTrue(gate.accepts(second, null, 1, config))
        assertEquals(config, second.configuration)
        assertNotSame(config, second.configuration)
    }
}
