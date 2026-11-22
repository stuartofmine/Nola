package com.nola.ui.screen.chat

import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.runtime.snapshots.SnapshotStateObserver
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class ConversationComposerDraftsTest {
    @Test fun starterPromptKeepsExpertIdentityAndDoesNotDuplicateOnRetap() {
        val draft = ConversationComposerDraft()
        val expert = InlineExpertMention(0, "@专家", "expert", 2)
        draft.setContent("@专家 帮我整理\n", listOf(expert))
        repeat(2) { draft.insertSuggestion("Plan today") }
        assertEquals("@专家 帮我整理\nPlan today", draft.text.value)
        assertEquals(listOf(expert), draft.mentions)
        assertEquals(draft.text.value.length, draft.caret)
    }

    @Test fun retryRestoresOnlyAnEmptyComposerAndPreservesAnEditedMentionAndCaret() {
        val draft = ConversationComposerDraft()
        draft.restoreRetryTextIfEmpty("Failed input")
        assertEquals("Failed input", draft.text.value)
        assertEquals(draft.text.value.length, draft.caret)
        val expert = InlineExpertMention(0, "@专家", "expert", 2)
        draft.setContent("@专家 新草稿", listOf(expert))
        draft.caret = 4
        draft.restoreRetryTextIfEmpty("Failed input")
        assertEquals("@专家 新草稿", draft.text.value)
        assertEquals(listOf(expert), draft.mentions)
        assertEquals(4, draft.caret)

        draft.text.value = ""
        val attachment = AndroidChatAttachment(name = "notes", typeLabel = "text", sizeBytes = 3,
            content = "abc", extractionLabel = "text", contentDigest = "", uri = "content://notes", mimeType = "text/plain")
        draft.attachments.value = listOf(attachment)
        draft.restoreRetryTextIfEmpty("Failed input")
        assertEquals("", draft.text.value)
        assertEquals(listOf(attachment), draft.attachments.value)
    }

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
    @Test fun blockedBeforeModelRestoresOriginalDraftButNeverOverwritesNewEdits() = runBlocking {
        val drafts=ConversationComposerDrafts()
        drafts.forConversation("a").text.value="Original input"
        val ticket=drafts.prepareSubmission(drafts.captureSubmission(drafts.ownerKey("a")))
        assertTrue(drafts.submitted(ticket))
        assertTrue(drafts.restoreBlockedSubmission(ticket,"a"))
        assertEquals("Original input",drafts.forConversation("a").text.value)
        assertFalse(drafts.restoreBlockedSubmission(ticket,"a"))
        drafts.forConversation("b").text.value="Submitted"
        val other=drafts.prepareSubmission(drafts.captureSubmission(drafts.ownerKey("b")))
        assertTrue(drafts.submitted(other));drafts.forConversation("b").text.value="Newer draft"
        assertFalse(drafts.restoreBlockedSubmission(other,"b"))
        assertEquals("Newer draft",drafts.forConversation("b").text.value)
    }
}
