// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** Actual ordinary composer, canonical admission and real provider; no simulated replies. */
class ConversationMentionProductUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun explicitExpertRepliesAndNewConversationClearsSelection() {
        val model = InstrumentationRegistry.getArguments().getString("mentionRealModel")
        assumeTrue(!model.isNullOrBlank())
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(15_000) { compose.onAllNodesWithContentDescription(context.getString(R.string.chat_new_chat)).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
            compose.onNodeWithTag("chat.modelPicker").performClick()
            compose.onNodeWithText(requireNotNull(model), substring = false).performClick()
            // Do not prime the directory by opening the expert picker first.
            compose.onNodeWithTag("chat.controls.permission").assertDoesNotExist()
            val input = compose.onNodeWithTag("chat.composer.input")
            input.performClick().performTextInput("＠Arith")
            compose.waitUntil(15_000) { compose.onAllNodesWithText("Arithmetic QA", substring = false).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("Arithmetic QA", substring = false).performClick()
            input.assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
            compose.onNodeWithText("Arithmetic QA ×").assertIsDisplayed()
            input.performTextInput("Compute 41 times 17. Reply ANDROID_REAL= followed by the result. Do not use tools.")
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).performClick()
            compose.waitUntil(180_000) { compose.onAllNodesWithText("ANDROID_REAL=697", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("ANDROID_REAL=697", substring = true).assertIsDisplayed()
            repeat(2) {
                compose.onNodeWithTag("chat.topbar.more").performClick()
                compose.onNodeWithTag("chat.expert.sessions.open").performClick()
                compose.onNodeWithTag("chat.expert.sessions.list").assertIsDisplayed()
                compose.onNode(hasText("Arithmetic QA") and hasAnyAncestor(hasTestTag("chat.expert.sessions.list")))
                    .performClick()
                compose.onNodeWithTag("chat.expert.sessions.transcript").assertIsDisplayed()
                compose.onNodeWithTag("chat.expert.sessions.back").performClick()
                compose.onNodeWithTag("chat.expert.sessions.list").assertIsDisplayed()
                compose.onNodeWithText(context.getString(R.string.common_done)).performClick()
                compose.onNodeWithTag("chat.expert.sessions").assertDoesNotExist()
                compose.onNodeWithText("ANDROID_REAL=697", substring = true).assertIsDisplayed()
            }
            compose.onNodeWithTag("chat.conversation.back").performClick()
            compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
            compose.onNodeWithText("Arithmetic QA ×").assertDoesNotExist()
        }
    }
}
