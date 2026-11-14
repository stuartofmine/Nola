// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.data.repository.ExpertChildConversation
import com.nola.data.repository.ExpertChildConversationPage
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Real Compose disclosure/pagination controls, isolated in-memory transport; no model or database. */
class ExpertChildTreeInteractionTest {
    @get:Rule val compose = createComposeRule()

    @Test fun disclosureIsLazyAndFailedPaginationPreservesRowsAndCanRefresh() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val calls = mutableListOf<Int>()
        var opened: ExpertHistoryTarget? = null
        val child = ExpertChildConversation("child", 1, "Frozen expert name", "member", "agent", null, false, false, 1, 2, 3)
        compose.setContent { MaterialTheme {
            ExpertChildTree("parent", load = { offset ->
                calls += offset
                if (offset > 0) error("Isolated offline fixture")
                ExpertChildConversationPage(version = 1, conversationId = "parent", roomId = "room", roomRevision = 1,
                    children = listOf(child), hasMore = true, nextOffset = 1)
            }, onOpen = { opened = it })
        } }
        compose.runOnIdle { assertTrue(calls.isEmpty()) }
        compose.onNodeWithTag("chat.expertHistory.parent").performClick()
        compose.waitUntil { compose.onAllNodesWithTag("chat.expertChild.child").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("chat.expertChild.child").performClick()
        compose.runOnIdle { assertEquals("parent", opened?.parentId); assertFalse(opened!!.child.isCurrent) }
        compose.onNodeWithText(context.getString(R.string.expert_child_more)).performClick()
        compose.waitUntil { compose.onAllNodesWithText(context.getString(R.string.expert_child_load_failed)).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("chat.expertChild.child").assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.expert_child_retry)).performClick()
        compose.waitUntil { calls.size >= 3 }
        compose.runOnIdle { assertEquals(listOf(0, 1, 0), calls) }
        compose.onNodeWithTag("chat.expertHistory.parent").performClick()
        compose.onNodeWithTag("chat.expertChild.child").assertDoesNotExist()
    }
}
