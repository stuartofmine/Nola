package com.nola.ui.screen.chat

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.core.AndroidChildPermissions
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import java.io.File

/** Two phases: the runner force-stops the isolated app between these tests. */
class RealEmbeddedChildPermissionUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun startChildAndLeaveItsActualPermissionPending() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
        val secure = SecureTokenStore(context)
        AndroidApplicationRuntime(context).use { app ->
            runBlocking {
                PreferenceDataStore(context, secure).apply {
                    setOnboardingCompleted()
                    setAgentModeEnabled(true)
                    setAgentPermissionMode(com.nola.core.AndroidAgentPermissionMode.Cautious)
                }
                MemoryProfileDataStore(context, secure, app).saveAgentLlmConfig(
                    config.getString("endpoint"), config.getString("apiKey"),
                    config.getString("model"), config.getString("protocol"))
            }
        }
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(30_000) {
                runCatching { compose.onAllNodesWithContentDescription(context.getString(R.string.tab_settings)).fetchSemanticsNodes().isNotEmpty() }.getOrDefault(false)
            }
            compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
            compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
            compose.waitUntil(30_000) { compose.onAllNodesWithTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model")).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText(context.getString(R.string.models_use)).performScrollTo().performClick()
            compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
            compose.onNodeWithTag("chat.conversations.new").performClick()
            compose.waitUntil(15_000) { compose.onAllNodesWithTag("chat.composer.input").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag("chat.composer.input").performTextInput(
                "Delegate exactly one general subagent to read the Android clipboard using the available clipboard tool and report its exact text. Use permission_mode ask. Wait for its actual result; do not read the clipboard yourself or invent its content.")
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performClick()
            compose.waitUntil(240_000) { AndroidChildPermissions.pending.value.isNotEmpty() }
            val pending = AndroidChildPermissions.pending.value.single()
            check(pending.request.toolName == "device_clipboard_read")
            File(context.filesDir, "embedded-child-pending.json").writeText(JSONObject()
                .put("parent", pending.parentConversationId).put("child", pending.childConversationId)
                .put("requestId", pending.request.requestId).toString())
            // Keep the Activity alive: the external runner kills the actual process.
            java.util.concurrent.CountDownLatch(1).await(120, java.util.concurrent.TimeUnit.SECONDS)
            error("External force-stop did not arrive after permission readiness")
        }
    }

    @Test fun coldRestartRestoresActualChildPermissionAndCompletes() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val previous = JSONObject(File(context.filesDir, "embedded-child-pending.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                (activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
                    .setPrimaryClip(ClipData.newPlainText("isolated QA", "CHILD-COLD-CLIPBOARD-842"))
            }
            val parentTag = "chat.conversation." + previous.getString("parent")
            compose.waitUntil(30_000) { compose.onAllNodesWithTag(parentTag).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag(parentTag).performClick()
            compose.waitUntil(180_000) { AndroidChildPermissions.pending.value.any {
                it.parentConversationId == previous.getString("parent") && it.childConversationId == previous.getString("child")
            } }
            val restored = AndroidChildPermissions.pending.value.single {
                it.childConversationId == previous.getString("child")
            }
            check(restored.request.requestId == previous.getString("requestId")) {
                "Cold recovery replaced the pending permission identity"
            }
            compose.onNodeWithText(context.getString(R.string.permission_action_read_clipboard)).performScrollTo().assertIsDisplayed()
            File(context.filesDir, "embedded-child-cold-permission.png").outputStream().use { output ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                    .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
            }
            compose.onNodeWithText(context.getString(R.string.permission_action_read_clipboard)).performClick()
            compose.waitUntil(240_000) { compose.onAllNodesWithText("CHILD-COLD-CLIPBOARD-842", substring = true).fetchSemanticsNodes().isNotEmpty() }
            File(context.filesDir, "embedded-child-cold-ui.txt").writeText(compose.onRoot(useUnmergedTree = true).printToString())
        }
    }
}
