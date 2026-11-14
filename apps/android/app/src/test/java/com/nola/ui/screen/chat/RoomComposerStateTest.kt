package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import com.nola.data.repository.RoomSubmission
import com.nola.data.repository.RoomSubmissionInput

class RoomComposerStateTest {
    private fun submission(id: String, revision: Long = 1, state: String = "accepted") = RoomSubmission(
        RoomSubmissionInput("room", id, 1, "search 当前", null, 1), state, null, null, revision, null, null)

    @Test fun latePollPreservesNewMessagesAndTerminalRevision() {
        val completed = submission("one", 5, "completed")
        val newer = submission("two")
        assertEquals(listOf(completed, newer), mergeRoomSubmissions("room", listOf(completed, newer), listOf(submission("one"))))
        assertEquals(listOf(completed), mergeRoomSubmissions("room", listOf(submission("one")), listOf(completed)))
    }

    @Test fun foreignOwnerAndChangedImmutableInputCannotReplaceReceipt() {
        val current = submission("one")
        val foreign = current.copy(input = current.input.copy(roomId = "other"), revision = 9)
        val changed = current.copy(input = current.input.copy(text = "changed"), revision = 9)
        assertEquals(listOf(current), mergeRoomSubmissions("room", listOf(current), listOf(foreign, changed)))
    }

    @Test fun onlyExplicitTrailingMentionOpensCandidates() {
        assertEquals("研", roomMentionQuery("请 @研"))
        assertEquals("", roomMentionQuery("@"))
        listOf("user@example.org", "search current", "当前搜索", "@Agent continue").forEach { assertNull(roomMentionQuery(it)) }
        assertEquals("请 ", removeRoomMentionQuery("请 @研"))
    }
}
