// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.domain.model.Conversation
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class ConversationListActionLabelsTest {
    @get:Rule val compose = createComposeRule()

    @Test fun newChatAndOptionsForAnUntitledConversationHaveDistinctActions() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val newChat = context.getString(R.string.chat_new_chat)
        var newChats = 0
        compose.setContent {
            ConversationListScreen(
                conversations = listOf(Conversation(id = "untitled")),
                onConversationSelected = {},
                onNewConversation = { newChats++ },
                onDeleteConversation = {},
                search = "",
                onSearchChange = {},
                listState = rememberLazyListState(),
                previews = emptyMap(),
                activity = emptyMap(),
                onLoadPreview = {}
            )
        }
        compose.onAllNodesWithContentDescription(newChat).assertCountEquals(1)
        compose.onNodeWithContentDescription(newChat).performClick()
        compose.runOnIdle { assertEquals(1, newChats) }
        compose.onNodeWithContentDescription(context.getString(R.string.conversation_more_options, newChat)).performClick()
        compose.onNodeWithText(context.getString(R.string.chat_rename_conversation)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.common_delete)).assertIsDisplayed()
        compose.runOnIdle { assertEquals(1, newChats) }
    }
}
