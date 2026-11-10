package com.nola.ui.screen.chat

import androidx.compose.runtime.saveable.SaverScope
import org.junit.Assert.*
import org.junit.Test

class ConversationUiContractTest {
    @Test fun pickerResultRetainsOriginalConversationAfterActivityRecreation() {
        for (owner in listOf(null, "a", "new", "pickerOwner")) {
            val store = ConversationComposerDrafts()
            store.forConversation(owner).text.value = "original"
            store.beginPicker(owner)
            val saved = with(ConversationComposerDrafts.Saver) {
                object : SaverScope {
                    override fun canBeSaved(value: Any): Boolean = true
                }.save(store)
            }
            val restored = ConversationComposerDrafts.Saver.restore(requireNotNull(saved))!!
            val other = restored.forConversation("other")
            other.text.value = "selected after returning"
            val target = restored.consumePicker()
            assertSame(restored.forConversation(owner), target)
            assertEquals("original", target!!.text.value)
            target.notice.value = "Imported"
            assertNull(other.notice.value)
            assertNull(restored.consumePicker())
        }
    }

    @Test fun layoutAtBottomDoesNotResumeReadingHistory() {
        assertFalse(followAfterViewportChange(false, false, false, true, false))
        assertTrue(followAfterViewportChange(true, false, false, false, false))
        assertFalse(followAfterViewportChange(true, true, true, false, false))
        assertTrue(followAfterViewportChange(false, true, false, true, false))
        assertFalse(followAfterViewportChange(false, true, false, true, true))
    }

    @Test fun draftsAndAsyncImportCompletionStayWithTheirConversation() {
        val store = ConversationComposerDrafts()
        val a = store.forConversation("a")
        a.text.value = "first draft"
        a.importing.value = true
        val b = store.forConversation("b")
        b.text.value = "second draft"
        a.notice.value = "Import finished"
        a.importing.value = false
        assertEquals("first draft", store.forConversation("a").text.value)
        assertEquals("second draft", b.text.value)
        assertNull(b.notice.value)
        assertFalse(b.importing.value)
        assertNotSame(store.forConversation(null), b)
    }
}
