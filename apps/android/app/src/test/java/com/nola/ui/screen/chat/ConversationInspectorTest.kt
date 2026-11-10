package com.nola.ui.screen.chat

import org.junit.Assert.*
import org.junit.Test

class ConversationInspectorTest {
    @Test fun sameSelectionClosesAndDifferentSelectionReplaces() {
        assertEquals(ConversationInspector.Progress, toggleInspector(null, ConversationInspector.Progress))
        assertNull(toggleInspector(ConversationInspector.Progress, ConversationInspector.Progress))
        assertEquals(ConversationInspector.Resources, toggleInspector(ConversationInspector.Progress, ConversationInspector.Resources))
    }
}
