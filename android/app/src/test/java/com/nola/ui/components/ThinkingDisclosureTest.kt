package com.nola.ui.components

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ThinkingDisclosureTest {
    @Test fun completionDoesNotCollapseContentUnderTheReader() {
        assertTrue(thinkingExpandedAfterStatusChange(true, true, false))
        assertFalse(thinkingExpandedAfterStatusChange(false, true, false))
    }

    @Test fun nextRoundRespectsManualDisclosureChoice() {
        assertFalse(thinkingExpandedAfterStatusChange(false, false, true))
        assertTrue(thinkingExpandedAfterStatusChange(true, false, true))
        assertTrue(thinkingExpandedAfterStatusChange(false, false, false))
    }
}
