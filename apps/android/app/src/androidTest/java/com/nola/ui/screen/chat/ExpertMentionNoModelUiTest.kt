// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.data.repository.AgentRoomRepository
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Real composer and canonical directory, no provider fixture or model inference. */
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class ExpertMentionNoModelUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun atSuggestionSelectsUnjoinedExpertWithoutCreatingAnEmptyConversation() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        runBlocking { PreferenceDataStore(context, SecureTokenStore(context)).setOnboardingCompleted() }
        fun screenshot(name: String) {
            compose.waitForIdle()
            InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(200, 3000)
            java.io.File(context.filesDir, "expert-mention-$name.png").outputStream().use { output ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
            }
        }
        AndroidApplicationRuntime(context).use { runtime ->
            val rooms = AgentRoomRepository(runtime)
            val name = "测试工程师"
            val role = rooms.roleTemplates().templates.first()
            val directory = rooms.directory().takeIf { before -> before.agents.any { it.name == name } } ?: rooms.createAgent(name, buildJsonObject { put("selectedPromptTemplateID", role.id) }, rooms.directory(), "检查用例与回归风险")
            val created = directory.agents.single { it.name == name }
            val initialConversations = runtime.listConversations().map { it.id }.toSet()
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(15_000) { compose.onAllNodesWithContentDescription(context.getString(R.string.chat_new_chat)).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
                val input = compose.onNodeWithTag("chat.composer.input")
                compose.waitUntil(15_000) { runCatching { input.assertIsEnabled(); true }.getOrDefault(false) }
                input.performClick().performTextInput("请 看结果")
                input.performTextInputSelection(androidx.compose.ui.text.TextRange(1))
                input.performTextInput("@测试")
                compose.waitUntil(15_000) { compose.onAllNodesWithText(name, substring = false).fetchSemanticsNodes().isNotEmpty() }
                screenshot("candidates")
                compose.onNodeWithText(name, substring = false).performClick()
                input.assertTextContains("请@$name 看结果")
                screenshot("inline")
                input.performKeyInput { pressKey(androidx.compose.ui.input.key.Key.Backspace) }
                input.assertTextContains("请 看结果")
                input.performKeyInput { keyDown(androidx.compose.ui.input.key.Key.CtrlLeft); pressKey(androidx.compose.ui.input.key.Key.Z); keyUp(androidx.compose.ui.input.key.Key.CtrlLeft) }
                input.assertTextContains("请@$name 看结果")
                assertEquals(initialConversations, runtime.listConversations().map { it.id }.toSet())
                assertEquals(created.revision, rooms.directory().agents.single { it.id == created.id }.revision)
            }
        }
    }
}
