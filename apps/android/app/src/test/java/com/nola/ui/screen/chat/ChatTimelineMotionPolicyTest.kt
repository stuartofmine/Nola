package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class ChatTimelineMotionPolicyTest {
    @Test
    fun onlyAStableAppendSuffixCanReceiveAnEntrance() {
        assertEquals(listOf("m3"), ChatTimelineMotionPolicy.appendedMessageIds(listOf("m1", "m2"), listOf("m1", "m2", "m3")))
        assertEquals(emptyList<String>(), ChatTimelineMotionPolicy.appendedMessageIds(null, listOf("history")))
        assertEquals(emptyList<String>(), ChatTimelineMotionPolicy.appendedMessageIds(listOf("m2", "m3"), listOf("m1", "m2", "m3")))
        assertEquals(emptyList<String>(), ChatTimelineMotionPolicy.appendedMessageIds(listOf("m1", "m2"), listOf("replacement")))
    }

    @Test
    fun itemIdentityCannotBeReusedAcrossConversationSwitches() {
        assertEquals(
            ChatTimelineMotionPolicy.ItemKey("conversation-a", "same-message-id"),
            ChatTimelineMotionPolicy.itemKey("conversation-a", "same-message-id"),
        )
        org.junit.Assert.assertNotEquals(
            ChatTimelineMotionPolicy.itemKey("conversation-a", "same-message-id"),
            ChatTimelineMotionPolicy.itemKey("conversation-b", "same-message-id"),
        )
    }
}
