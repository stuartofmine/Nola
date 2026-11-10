package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import java.io.File
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Real app, encrypted paired Host, and provider. No synthetic task results. */
@RunWith(AndroidJUnit4::class)
class RealAndroidRemoteTaskUiTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun pairThroughUiRunChildAndReadRemoteProgress() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10) { "Requires isolated Android test user" }
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        val prefs = PreferenceDataStore(context, SecureTokenStore(context))
        runBlocking { prefs.setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
                compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
                compose.onNode(hasText(context.getString(R.string.models_web_host)) and hasClickAction()).performClick()
                if (!config.optBoolean("alreadyPaired")) {
                compose.onAllNodes(hasText(context.getString(R.string.models_web_host)) and hasClickAction()).onLast().performScrollTo().performClick()
                compose.onNodeWithText(context.getString(R.string.models_host_url)).performScrollTo().performTextReplacement(config.getString("url"))
                compose.onNodeWithText(context.getString(R.string.models_access_token)).performScrollTo().performTextReplacement(config.getString("token"))
                compose.onNodeWithText(context.getString(R.string.models_save_and_verify)).performClick()
                compose.waitUntil(60_000) {
                    compose.onAllNodesWithText(context.getString(R.string.models_access_token)).fetchSemanticsNodes().isEmpty()
                }
                }
                activateSavedHost()
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                compose.onNodeWithTag("chat.conversations.new").performClick()
                compose.onNodeWithTag("chat.composer.input").performTextInput(
                    "Please actually delegate one read_only subagent to compute 17 times 23 using only mental arithmetic, without tools or files. Wait for its actual completion, then report the result and decode QU5EUk9JRF9SRU1PVEVfQ0hJTERfMzkx as a final marker.")
                compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performClick()
                compose.waitUntil(240_000) {
                    compose.onAllNodesWithText("ANDROID_REMOTE_CHILD_391", substring = true).fetchSemanticsNodes().isNotEmpty()
                }
                screenshot("remote-task-answer.png")
                openChildResultAndReturn()
                compose.onNodeWithTag("chat.topbar.more").performClick()
                compose.onNodeWithText(context.getString(R.string.inspector_progress)).performClick()
                compose.waitUntil(30_000) {
                    compose.onAllNodesWithText(context.getString(R.string.inspector_no_plan)).fetchSemanticsNodes().isNotEmpty()
                }
                compose.waitForIdle()
                compose.onAllNodesWithText(context.getString(R.string.inspector_progress_unavailable)).assertCountEquals(0)
                screenshot("remote-progress.png")
                File(context.filesDir, "real-remote-ui-result.json").writeText(JSONObject().put("actualUiPairing",true).put("answerMarkerVisible",true).put("remoteProgressNoError",true).toString())
            } catch (error: Throwable) {
                screenshot("remote-ui-failure.png")
                File(context.filesDir, "remote-ui-failure.txt").writeText((error.stackTraceToString() + "\n" + runCatching { compose.onRoot(useUnmergedTree = true).printToString() }.getOrDefault("UI hierarchy unavailable")).replace(config.getString("token"), "<redacted>"))
                throw error
            }
        }
    }
    private fun openChildResultAndReturn() {
        val cards = SemanticsMatcher("canonical task card") {
            it.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)?.startsWith("chat.task.card.") == true
        }
        compose.waitUntil(30_000) { compose.onAllNodes(cards).fetchSemanticsNodes().isNotEmpty() }
        compose.onAllNodes(cards).onFirst().performScrollTo().performClick()
        val results = SemanticsMatcher("exact child result entry") {
            it.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)?.startsWith("chat.task.detail.open.") == true
        }
        compose.onAllNodes(results).onFirst().performScrollTo().performClick()
        compose.waitUntil(30_000) { compose.onAllNodesWithTag("chat.task.detail").fetchSemanticsNodes().isNotEmpty() }
        compose.waitUntil(30_000) {
            compose.onAllNodes(hasText("391", substring = true) and hasAnyAncestor(hasTestTag("chat.task.detail")))
                .fetchSemanticsNodes().isNotEmpty()
        }
        compose.onAllNodes(hasTestTag("chat.composer.input") and hasAnyAncestor(hasTestTag("chat.task.detail"))).assertCountEquals(0)
        screenshot("remote-child-readonly.png")
        compose.onNodeWithTag("chat.task.detail.back").performClick()
        compose.onNodeWithTag("chat.composer.input").assertExists()
        screenshot("remote-child-returned-parent.png")
    }

    @Test fun coldRemoteChildHistoryAndResult() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use {
            val conversationTag = "chat.conversation.${config.getString("conversationId")}"
            compose.waitUntil(60_000) {
                compose.onAllNodesWithTag("chat.topbar.more").fetchSemanticsNodes().isNotEmpty() ||
                    compose.onAllNodesWithTag(conversationTag).fetchSemanticsNodes().isNotEmpty()
            }
            if (compose.onAllNodesWithTag(conversationTag).fetchSemanticsNodes().isNotEmpty())
                compose.onNodeWithTag(conversationTag).performScrollTo().performClick()
            compose.waitUntil(60_000) {
                compose.onAllNodesWithText("ANDROID_REMOTE_CHILD_391", substring = true).fetchSemanticsNodes().isNotEmpty()
            }
            openChildResultAndReturn()
            screenshot("remote-child-cold-history.png")
        }
    }

    @Test fun coldRemoteHistoryAndAuthoritativeGoalProgress() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                val conversationTag = "chat.conversation.${config.getString("conversationId")}"
                compose.waitUntil(60_000) {
                    compose.onAllNodesWithTag("chat.topbar.more").fetchSemanticsNodes().isNotEmpty() ||
                        compose.onAllNodesWithTag(conversationTag).fetchSemanticsNodes().isNotEmpty()
                }
                if (compose.onAllNodesWithTag(conversationTag).fetchSemanticsNodes().isNotEmpty())
                    compose.onNodeWithTag(conversationTag).performScrollTo().performClick()
                compose.waitUntil(30_000) { compose.onAllNodesWithTag("chat.topbar.more").fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithTag("chat.topbar.more").performClick()
                compose.onNodeWithText(context.getString(R.string.inspector_progress)).performClick()
                compose.waitUntil(30_000) {
                    compose.onAllNodesWithText(config.getString("objective"), substring = true).fetchSemanticsNodes().isNotEmpty()
                }
                compose.onNodeWithContentDescription(context.getString(R.string.common_refresh)).performClick()
                compose.waitUntil(30_000) {
                    compose.onAllNodesWithText(config.getString("goalStatus"), substring = false).fetchSemanticsNodes().isNotEmpty()
                }
                compose.onNodeWithText(config.getString("requirement"), substring = true).performScrollTo().assertIsDisplayed()
                compose.onAllNodesWithText(context.getString(R.string.inspector_progress_unavailable)).assertCountEquals(0)
                screenshot("remote-goal-cold.png")
                File(context.filesDir, "real-remote-goal-result.json").writeText(JSONObject()
                    .put("actualColdUiGoal",true).put("objective",config.getString("objective"))
                    .put("status",config.getString("goalStatus")).toString())
            } catch (error: Throwable) {
                screenshot("remote-goal-failure.png")
                File(context.filesDir, "remote-goal-failure.txt").writeText(compose.onRoot(useUnmergedTree = true).printToString())
                throw error
            }
        }
    }

    @Test fun selectDefaultHostThroughUi() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10)
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
            compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
            compose.onNode(hasText(context.getString(R.string.models_web_host)) and hasClickAction()).performClick()
            activateSavedHost()
        }
    }

    @Test fun coldNewConversationRestoresSelectedHostWithoutSending() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10)
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
            compose.onNodeWithTag("chat.conversations.new").performClick()
            compose.waitUntil(30_000) { compose.onAllNodesWithText("Nola Web Host", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.onAllNodesWithText(context.getString(R.string.chat_no_model_configured)).assertCountEquals(0)
            compose.onNodeWithTag("chat.composer.input").performTextInput("UNSENT_DEFAULT_HOST_CHECK")
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled()
            screenshot("cold-default-host.png")
        }
    }

    @Test fun existingChildDetailIsReadOnlyAndReturnsToParentDraft() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(android.os.Process.myUid() / 100000 >= 10)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use {
            val parentTag = "chat.conversation.${config.getString("conversationId")}"
            compose.waitUntil(60_000) { compose.onAllNodesWithTag(parentTag).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag(parentTag).performScrollTo().performClick()
            compose.onNodeWithTag("chat.composer.input").performTextReplacement("UNSENT_PARENT_DETAIL_DRAFT")
            InstrumentationRegistry.getInstrumentation().uiAutomation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK)
            val task = config.getString("taskId")
            compose.waitUntil(30_000) {
                runCatching { compose.onNodeWithTag("chat-transcript-list").performScrollToNode(hasTestTag("chat.task.card.$task")) }.isSuccess
            }
            compose.onNodeWithTag("chat.task.card.$task").performClick()
            compose.onNodeWithTag("chat.task.detail.open.$task").performClick()
            compose.waitUntil(30_000) { compose.onAllNodes(hasText("391", substring = true) and hasAnyAncestor(hasTestTag("chat.task.detail.message.${config.getString("childAssistantId")}"))).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag("chat.task.detail").assertIsDisplayed()
            compose.onAllNodes(hasTestTag("chat.composer.input") and hasAnyAncestor(hasTestTag("chat.task.detail"))).assertCountEquals(0)
            screenshot("existing-child-readonly.png")
            compose.onNodeWithTag("chat.task.detail.back").performClick()
            compose.onNodeWithTag("chat.composer.input").assertTextEquals("UNSENT_PARENT_DETAIL_DRAFT")
            compose.onNodeWithTag("chat.task.card.$task").assertIsDisplayed()
            screenshot("existing-child-return-parent.png")
        }
    }

    private fun activateSavedHost() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val row = hasTestTag("models.row.web-host")
        compose.waitUntil(30_000) { compose.onAllNodes(row).fetchSemanticsNodes().isNotEmpty() }
        val use = hasText(context.getString(R.string.models_use)) and hasAnyAncestor(row)
        if (compose.onAllNodes(use).fetchSemanticsNodes().isNotEmpty()) compose.onNode(use).performScrollTo().performClick()
        compose.waitUntil(30_000) { compose.onAllNodes(hasText(context.getString(R.string.models_selected)) and hasAnyAncestor(row)).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        File(instrumentation.targetContext.filesDir, name).outputStream().use {
            instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG,100,it)
        }
    }
}
