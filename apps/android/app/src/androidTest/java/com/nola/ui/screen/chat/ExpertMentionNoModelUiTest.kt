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
class ExpertMentionNoModelUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun atSuggestionSelectsUnjoinedExpertWithoutCreatingAnEmptyConversation() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        runBlocking { PreferenceDataStore(context, SecureTokenStore(context)).setOnboardingCompleted() }
        AndroidApplicationRuntime(context).use { runtime ->
            val rooms = AgentRoomRepository(runtime)
            val name = "Mention no-model " + java.util.UUID.randomUUID().toString().take(8)
            val role = rooms.roleTemplates().templates.first()
            val directory = rooms.createAgent(name, buildJsonObject { put("selectedPromptTemplateID", role.id) }, rooms.directory(), "Fixture responsibility")
            val created = directory.agents.single { it.name == name }
            val initialConversations = runtime.listConversations().map { it.id }.toSet()
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(15_000) { compose.onAllNodesWithContentDescription(context.getString(R.string.chat_new_chat)).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithContentDescription(context.getString(R.string.chat_new_chat)).performClick()
                val input = compose.onNodeWithTag("chat.composer.input")
                compose.waitUntil(15_000) { runCatching { input.assertIsEnabled(); true }.getOrDefault(false) }
                input.performClick().performTextInput("@Mention")
                compose.waitUntil(15_000) { compose.onAllNodesWithText(name, substring = false).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithText(name, substring = false).performClick()
                compose.onNodeWithText("$name ×").assertIsDisplayed()
                input.assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText,
                    androidx.compose.ui.text.AnnotatedString("")))
                assertEquals(initialConversations, runtime.listConversations().map { it.id }.toSet())
                assertEquals(created.revision, rooms.directory().agents.single { it.id == created.id }.revision)
            }
        }
    }
}
