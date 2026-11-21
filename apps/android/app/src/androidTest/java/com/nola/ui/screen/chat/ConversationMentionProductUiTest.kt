// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.repository.AgentRoomRepository
import dagger.hilt.android.EntryPointAccessors
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** Actual ordinary composer, canonical admission and real provider; no simulated replies. */
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class ConversationMentionProductUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun explicitExpertRepliesAndNewConversationClearsSelection() {
        val model = InstrumentationRegistry.getArguments().getString("mentionRealModel")
        assumeTrue(!model.isNullOrBlank())
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val deps = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java)
        val runtime = deps.runtime()
        val rooms = AgentRoomRepository(runtime)
        val name = "Arithmetic QA"
        val id = runBlocking {
            val config = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
            check(config.getString("model") == model)
            applyRealModelFixture(context, runtime, config)
            deps.preferences().setOnboardingCompleted()
            val repository = deps.modelRepository()
            val initial = repository.listConfiguredModels().single { it.id == model }
            if (!initial.executionCapabilitiesDeclared || !initial.executionCapabilities.toolCalls)
                repository.updateModelExecutionCapabilities(initial, initial.executionCapabilities.copy(toolCalls = true))
            val selected = repository.listConfiguredModels().single { it.id == model }
            deps.preferences().saveDefaultModelSelection(selected)
            val before = rooms.directory()
            val directory = if (before.agents.any { it.name == name }) before else rooms.createAgent(name, buildJsonObject {
                put("selectedPromptTemplateID", rooms.roleTemplates().templates.first().id)
            }, before, "Compute and cross-check arithmetic when the main agent delegates it.")
            directory.agents.single { it.name == name }.id
        }
        val originalOwners = runtime.listConversations().map { it.id }.toSet()
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(15_000) { compose.onAllNodesWithContentDescription(context.getString(R.string.chat_new_chat)).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
            val input = compose.onNodeWithTag("chat.composer.input")
            input.performClick().performTextInput("@Arith")
            compose.waitUntil(15_000) { compose.onAllNodesWithText("Arithmetic QA", substring = false).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("Arithmetic QA", substring = false).performClick()
            input.assertTextContains("@Arithmetic QA")
            val instruction = " Ask the mentioned expert to compute 41 times 17. Return ANDROID_REAL= followed by their result."
            input.performTextInput(instruction)
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).performClick()
            compose.waitUntil(180_000) { compose.onAllNodesWithText("ANDROID_REAL=697", substring = true).fetchSemanticsNodes().isNotEmpty() }
            val owner = runtime.listConversations().single { row -> row.id !in originalOwners && rooms.forConversation(row.id)?.room?.room?.participants?.any { it.agentId == id } == true }.id
            val children = rooms.childConversations(owner)
            val originalChild = children.children.single { it.agentId == id && it.isCurrent && it.messageCount > 0 }
            val originalChildInputs = runtime.loadConversationProjectionV1(originalChild.conversationId).messages
                .filter { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User }.map { it.messageId }.toSet()
            check(originalChildInputs.isNotEmpty())
            val message = runtime.loadConversationProjectionV1(owner).messages.single { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User }
            val raw = (message.parts.single() as com.nola.domain.projection.ConversationProjectionV1.Part.Historical).projection
            assertEquals(id, historyMentions("@$name$instruction", raw.expertMentions).single().agentId)
            input.assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
            compose.waitForIdle()
            File(context.filesDir, "expert-mention-real.png").outputStream().use { file ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, file)
            }
            // Change only the instruction. The original @ reference in this message remains bound to its frozen member.
            val editAction = compose.onNodeWithTag("chat.message.edit." + message.messageId)
            compose.waitUntil(15_000) { runCatching { editAction.assertIsEnabled(); true }.getOrDefault(false) }
            editAction.performClick()
            val edit = compose.onNodeWithTag("chat.message.editor.input")
            compose.waitUntil(15_000) { compose.onAllNodesWithTag("chat.message.editor.input").fetchSemanticsNodes().isNotEmpty() }
            edit.assertTextContains("@$name$instruction")
            edit.performTextInputSelection(androidx.compose.ui.text.TextRange(name.length + 1, name.length + 1 + instruction.length))
            val replacement = " Ask this expert to calculate 19 times 3 and respond with EDIT_REAL= followed by the answer."
            edit.performTextInput(replacement)
            compose.onNodeWithTag("chat.message.editor.submit").performClick()
            compose.waitUntil(180_000) { compose.onAllNodesWithText("EDIT_REAL=57", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.waitUntil(10_000) { compose.onAllNodesWithTag("chat.message.editor.input").fetchSemanticsNodes().isEmpty() }
            compose.onNodeWithText("ANDROID_REAL=697", substring = true).assertDoesNotExist()
            compose.onNodeWithText("41 times 17", substring = true).assertDoesNotExist()
            val changed = runtime.loadConversationProjectionV1(owner).messages.single { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User }
            check(changed.messageId != message.messageId)
            val changedRaw = (changed.parts.single() as com.nola.domain.projection.ConversationProjectionV1.Part.Historical).projection
            assertEquals(id, historyMentions("@$name$replacement", changedRaw.expertMentions).single().agentId)
            val activeChild = rooms.childConversations(owner).children.single { it.agentId == id && it.isCurrent }
            val childMessages = runtime.loadConversationProjectionV1(activeChild.conversationId).messages
            check(childMessages.any { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User } &&
                childMessages.none { it.messageId in originalChildInputs }) {
                "Rewinding an edited user message must also retire the abandoned delegation in its child's active history"
            }
            val expertAnswer = com.nola.domain.projection.CanonicalConversationProjector.project(runtime.loadConversationProjectionV1(activeChild.conversationId))
                .filter { it.role == com.nola.domain.model.ChatRole.assistant }
            check(expertAnswer.any { it.text.contains("57") })
            File(context.filesDir, "expert-mention-real-edit.png").outputStream().use { file ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, file)
            }
            // Rejoining freezes a new context. The original @ still opens its original read-only history.
            val directory = rooms.directory()
            val agent = directory.agents.single { it.id == id }
            val updated = rooms.updateAgent(agent, agent.name, "Arithmetic QA revision " + agent.revision, directory)
            rooms.rejoin(updated.agents.single { it.id == id }, checkNotNull(rooms.forConversation(owner)).room)
            val oldContext = rooms.childConversations(owner).children.single { it.conversationId == activeChild.conversationId }
            check(!oldContext.isCurrent)
            compose.onNodeWithText("@$name$replacement", useUnmergedTree = true)
                .performTouchInput { click(androidx.compose.ui.geometry.Offset(25f, 20f)) }
            val openHistory = context.getString(R.string.expert_child_history)
            compose.waitUntil(15_000) { compose.onAllNodesWithText(openHistory).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText(openHistory).performClick()
            compose.waitUntil(15_000) { compose.onAllNodesWithText("57", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText(context.getString(R.string.expert_child_read_only)).assertIsDisplayed()
            compose.onNodeWithText(context.getString(R.string.expert_child_continue)).assertDoesNotExist()
            File(context.filesDir, "expert-mention-real-retired.png").outputStream().use { file ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, file)
            }
            compose.onNode(hasContentDescription(context.getString(R.string.common_back)) and hasAnyAncestor(isDialog())).performClick()
            compose.onNodeWithTag("chat.conversation.back").performClick()
            compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
            compose.onNodeWithTag("chat.composer.input").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
        }
    }
}
