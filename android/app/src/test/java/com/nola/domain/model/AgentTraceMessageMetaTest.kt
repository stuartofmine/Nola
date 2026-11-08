package com.nola.domain.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentTraceMessageMetaTest {
    @Test
    fun visibleThinkingAndPhaseRoundTrip() {
        val meta = encodeAgentTraceProjectionForMeta(
            thinking = "  Inspecting the selected sources.  ",
            phase = "  searching  ",
        )

        assertEquals("Inspecting the selected sources.", decodeAgentThinkingFromMeta(meta))
        assertEquals("searching", decodeAgentTracePhaseFromMeta(meta))
        assertTrue(encodeAgentTraceProjectionForMeta(" ", null).isEmpty())
    }
}
