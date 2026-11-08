package com.nola.ui.screen.chat

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AnswerSourceLinkPolicyTest {
    @Test fun acceptsOnlyExplicitExternalWebLinks() {
        listOf("https://example.org/report#page=2", "HTTP://example.org/path?q=report").forEach {
            assertTrue(it, isExternalAnswerSource(it))
        }
        listOf("file:///data/private/report", "content://com.nola.fileprovider/report",
            "javascript:alert(1)", "intent://example.org", "/private/report", "//example.org",
            "https://", "https:example.org", "https://user:secret@example.org", "https://example.org/\n").forEach {
            assertFalse(it, isExternalAnswerSource(it))
        }
    }
}
