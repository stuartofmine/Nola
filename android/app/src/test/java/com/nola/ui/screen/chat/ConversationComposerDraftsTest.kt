package com.nola.ui.screen.chat

import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.runtime.snapshots.SnapshotStateObserver
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class ConversationComposerDraftsTest {
    @Test fun admittedDraftRetirementInvalidatesComposerObservationImmediately() = runBlocking {
        val drafts = ConversationComposerDrafts()
        drafts.forConversation("a").text.value = "submitted"
        val ticket = drafts.prepareSubmission(drafts.captureSubmission(drafts.ownerKey("a")))
        Snapshot.sendApplyNotifications()
        var invalidated = false
        val observer = SnapshotStateObserver { it() }
        observer.start()
        try {
            observer.observeReads("composer", { _: String -> invalidated = true }) {
                assertEquals("submitted", drafts.forConversation("a").text.value)
            }
            assertTrue(drafts.submitted(ticket))
            Snapshot.sendApplyNotifications()
            assertTrue(invalidated)
            assertEquals("", drafts.forConversation("a").text.value)
        } finally { observer.stop() }
    }

    @Test fun admissionKeepsNewEditAndOtherMemberDraft() = runBlocking {
        val drafts = ConversationComposerDrafts()
        drafts.forConversation("a").text.value = "submitted"
        drafts.forConversation("b").text.value = "b draft"
        val ticket = drafts.prepareSubmission(drafts.captureSubmission(drafts.ownerKey("a")))
        drafts.forConversation("a").text.value = "new a edit"
        assertFalse(drafts.submitted(ticket))
        assertEquals("new a edit", drafts.forConversation("a").text.value)
        assertEquals("b draft", drafts.forConversation("b").text.value)
    }
}
