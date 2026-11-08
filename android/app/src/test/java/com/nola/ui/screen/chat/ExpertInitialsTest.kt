package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class ExpertInitialsTest {
    @Test fun initialsAreStableForWhitespaceAndNonLatinNames() {
        assertEquals("AB", expertInitials("  alpha   beta gamma "))
        assertEquals("研", expertInitials("研究助手"))
        assertEquals("🤖A", expertInitials("🤖 alpha"))
        assertEquals("?", expertInitials("  "))
    }
}
