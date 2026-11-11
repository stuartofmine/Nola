package com.nola.ui.components

import org.junit.Assert.*
import org.junit.Test

class ToolDetailFormattingTest {
    @Test fun structuredArgumentsAreIndentedWithoutChangingPlainOutput() {
        assertTrue(formattedToolDetail("{\"path\":\"/tmp/test\",\"options\":{\"read\":true}}").contains("\n"))
        assertEquals("command output\n  aligned text", formattedToolDetail("command output\n  aligned text"))
        assertEquals("{incomplete", formattedToolDetail("{incomplete"))
    }

    @Test fun longMultilineOutputGetsAReaderEvenBelowCharacterLimit() {
        val excerpt = toolDetailExcerpt((1..30).joinToString("\n") { "line $it" })
        assertTrue(excerpt.truncated)
        assertTrue(excerpt.text.endsWith("…"))
        assertFalse(excerpt.text.contains("line 17"))
    }

    @Test fun shortOutputStaysCompleteAndLongSingleLineIsBounded() {
        assertEquals(ToolDetailExcerpt("done", false), toolDetailExcerpt("done"))
        assertEquals(2001, toolDetailExcerpt("x".repeat(3000)).text.length)
    }
}
