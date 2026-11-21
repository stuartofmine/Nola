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
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        val prefs = PreferenceDataStore(context, SecureTokenStore(context))
        runBlocking { prefs.setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                pairAndSelectHost(context, config)
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                compose.onNodeWithTag("chat.conversations.new").performClick()
                val input = readyComposer()
                if (config.has("expertName")) {
                    val name = config.getString("expertName")
                    input.performClick().performTextInput("@$name")
                    compose.waitUntil(20_000) { compose.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty() }
                    compose.onNodeWithText(name).performClick()
                    input.assertTextContains("@$name")
                }
                input.performTextInput(
                    " " + config.optString("prompt", "Please actually delegate one read_only subagent to compute 17 times 23 using only mental arithmetic, without tools or files. Wait for its actual completion, then report the result and decode QU5EUk9JRF9SRU1PVEVfQ0hJTERfMzkx as a final marker."))
                compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performClick()
                compose.waitUntil(240_000) {
                    check(compose.onAllNodesWithText("unknown variant", substring = true).fetchSemanticsNodes().isEmpty()) {
                        "Remote expert submission used an unrecognized Core command"
                    }
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
    private fun pairAndSelectHost(context: android.content.Context, config: JSONObject) {
        compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
        compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
        compose.onNode(hasText(context.getString(R.string.models_web_host)) and isSelectable()).performClick()
        if (!config.optBoolean("alreadyPaired")) {
            compose.onNodeWithTag(com.nola.ui.screen.models.MODEL_HOST_CONNECTION_TEST_TAG).performScrollTo().performClick()
            compose.onNodeWithTag("models.host.url").performScrollTo().performTextReplacement(config.getString("url"))
            val credential = compose.onNodeWithTag("models.host.token")
            credential.performScrollTo().performTextReplacement(config.getString("token"))
            credential.performImeAction()
            compose.waitUntil(60_000) {
                compose.onAllNodesWithTag("models.host.heading").fetchSemanticsNodes().isEmpty()
            }
        }
        activateSavedHost()
    }

    @Test fun ordinaryRemoteChatStartsGoalUsingHostDefault() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        val prefs = PreferenceDataStore(context, SecureTokenStore(context))
        runBlocking { prefs.setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                pairAndSelectHost(context, config)
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                compose.onNodeWithTag("chat.conversations.new").performClick()
                readyComposer().performTextInput("Please answer with only the product of 73 and 17.")
                compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).performClick()
                compose.waitUntil(120_000) { compose.onAllNodesWithText("1241", substring = true).fetchSemanticsNodes().isNotEmpty() }
                screenshot("goal-default-chat.png")
                compose.onNodeWithTag("chat.topbar.more").performClick()
                compose.onNodeWithText(context.getString(R.string.inspector_progress)).performClick()
                val objective = "Wait for the user to review the Android result before completion."
                compose.waitUntil(30_000) { compose.onAllNodesWithText(context.getString(R.string.goal_objective)).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithText(context.getString(R.string.goal_objective)).performScrollTo().performTextReplacement(objective)
                compose.onNodeWithText(context.getString(R.string.goal_start)).performScrollTo().performClick()
                compose.waitUntil(40_000) {
                    check(compose.onAllNodesWithText("no saved model selection", substring = true).fetchSemanticsNodes().isEmpty()) {
                        "Host rejected Goal in the ordinary remote conversation because its model choice was inherited"
                    }
                    compose.onAllNodesWithText(context.getString(R.string.goal_pause)).fetchSemanticsNodes().isNotEmpty()
                }
                screenshotGoalControls("goal-default-started.png", R.string.goal_pause, R.string.canonical_running)
                compose.onNodeWithText(context.getString(R.string.goal_pause)).performScrollTo().performClick()
                compose.waitUntil(90_000) { compose.onAllNodesWithText(context.getString(R.string.goal_resume)).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithText(context.getString(R.string.goal_resume)).performScrollTo().assertIsDisplayed()
                compose.onAllNodesWithText(objective).onFirst().performScrollTo().assertIsDisplayed()
                screenshotGoalControls("goal-default-paused.png", R.string.goal_resume, R.string.scheduled_paused)
            } catch (failure: Throwable) {
                screenshot("goal-default-failure.png")
                File(context.filesDir, "goal-default-failure.txt").writeText((failure.stackTraceToString() + "\n" +
                    compose.onAllNodes(isRoot(), useUnmergedTree = true).printToString()).replace(config.getString("token"), "<redacted>"))
                throw failure
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
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use {
            openParent(config.getString("conversationId"))
            compose.waitUntil(60_000) {
                compose.onAllNodesWithText("ANDROID_REMOTE_CHILD_391", substring = true).fetchSemanticsNodes().isNotEmpty()
            }
            openChildResultAndReturn()
            screenshot("remote-child-cold-history.png")
        }
    }

    @Test fun coldRemoteHistoryAndAuthoritativeGoalProgress() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                openParent(config.getString("conversationId"))
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
                if (config.has("goalRunId")) {
                    val contract = hasTestTag("goal.contract.${config.getString("goalRunId")}")
                    compose.onNode(contract).performScrollTo().assertIsDisplayed()
                    val contractObjective = config.getString("contractObjective")
                    if (contractObjective != config.getString("objective"))
                        compose.onNode(contract).assert(hasAnyDescendant(hasText(contractObjective)))
                    if (config.optString("contractStatus") == "interrupted")
                        compose.onNode(contract).assert(hasAnyDescendant(hasText(context.getString(R.string.goal_step_interrupted))))
                    screenshot("remote-goal-contract.png")
                    InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                    compose.waitUntil(30_000) { compose.onAllNodesWithTag("chat.composer.input").fetchSemanticsNodes().isNotEmpty() }
                    compose.onAllNodesWithText("no source submission admission", substring = true).assertCountEquals(0)
                    compose.onAllNodesWithText("Store invariant failed", substring = true).assertCountEquals(0)
                }
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
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
            compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
            compose.onNode(hasText(context.getString(R.string.models_web_host)) and isSelectable()).performClick()
            activateSavedHost()
        }
    }

    @Test fun coldNewConversationRestoresSelectedHostWithoutSending() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
            compose.onNodeWithTag("chat.conversations.new").performClick()
            compose.waitUntil(30_000) { compose.onAllNodesWithText("Nola Web Host", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.onAllNodesWithText(context.getString(R.string.chat_no_model_configured)).assertCountEquals(0)
            readyComposer().performTextInput("UNSENT_DEFAULT_HOST_CHECK")
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled()
            screenshot("cold-default-host.png")
        }
    }

    @Test fun existingChildDetailIsReadOnlyAndReturnsToParentDraft() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-remote-e2e.json").readText())
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            openParent(config.getString("conversationId"))
            val input = readyComposer()
            input.performTouchInput { click() }
            input.performTextReplacement("UNSENT_PARENT_DETAIL_DRAFT")
            scenario.onActivity { activity ->
                androidx.core.view.WindowCompat.getInsetsController(activity.window, activity.window.decorView)
                    .hide(androidx.core.view.WindowInsetsCompat.Type.ime())
            }
            val task = config.getString("taskId")
            val card = hasTestTag("chat.task.card.$task")
            compose.waitUntil(30_000) {
                compose.onAllNodes(card).fetchSemanticsNodes().isNotEmpty() ||
                    runCatching { compose.onNodeWithTag("chat-transcript-list").performScrollToNode(card) }.isSuccess
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

    private fun openParent(conversationId: String) {
        val back = hasTestTag("chat.conversation.back")
        val list = hasTestTag("chat.conversations.list")
        compose.waitUntil(60_000) { compose.onAllNodes(back or list).fetchSemanticsNodes().isNotEmpty() }
        if (compose.onAllNodes(back).fetchSemanticsNodes().isNotEmpty()) compose.onNode(back).performClick()
        val target = hasTestTag("chat.conversation.$conversationId")
        compose.waitUntil(60_000) {
            runCatching { compose.onNode(list).performScrollToNode(target) }.isSuccess
        }
        compose.onNode(target).performClick()
    }

    private fun readyComposer(): SemanticsNodeInteraction {
        val input = hasTestTag("chat.composer.input") and isEnabled()
        compose.waitUntil(30_000) { compose.onAllNodes(input).fetchSemanticsNodes().isNotEmpty() }
        return compose.onNode(input)
    }

    private fun activateSavedHost() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val row = hasTestTag("models.row.web-host")
        compose.waitUntil(30_000) { compose.onAllNodes(row).fetchSemanticsNodes().isNotEmpty() }
        val use = hasText(context.getString(R.string.models_use)) and hasAnyAncestor(row)
        if (compose.onAllNodes(use).fetchSemanticsNodes().isNotEmpty()) compose.onNode(use).performScrollTo().performClick()
        compose.waitUntil(30_000) { compose.onAllNodes(hasText(context.getString(R.string.models_selected)) and hasAnyAncestor(row)).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun screenshotGoalControls(name: String, actionId: Int, statusId: Int) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val action = context.getString(actionId)
        val status = context.getString(statusId)
        val automation = instrumentation.uiAutomation
        compose.waitUntil(15_000) { compose.onAllNodes(hasText(action) and isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText(action).performScrollTo().assertIsDisplayed().assertIsEnabled()
        compose.waitUntil(15_000) {
            val nodes = java.util.ArrayDeque<android.view.accessibility.AccessibilityNodeInfo>()
            automation.rootInActiveWindow?.let(nodes::addLast)
            val labels = mutableSetOf<String>()
            while (nodes.isNotEmpty()) {
                val node = nodes.removeFirst()
                if (node.isVisibleToUser) {
                    node.text?.toString()?.let(labels::add)
                    node.contentDescription?.toString()?.let(labels::add)
                }
                for (index in 0 until node.childCount) node.getChild(index)?.let(nodes::addLast)
            }
            action in labels && status in labels && context.getString(R.string.goal_operation_pending) !in labels
        }
        // Compose semantics can update one frame before the system framebuffer.
        // Wait for accessibility updates to settle and for two actual display frames.
        automation.waitForIdle(250, 5_000)
        val rendered = java.util.concurrent.CountDownLatch(1)
        instrumentation.runOnMainSync {
            val choreographer = android.view.Choreographer.getInstance()
            choreographer.postFrameCallback { choreographer.postFrameCallback { rendered.countDown() } }
        }
        check(rendered.await(5, java.util.concurrent.TimeUnit.SECONDS)) { "The Goal state was not drawn" }
        screenshot(name)
    }

    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        File(instrumentation.targetContext.filesDir, name).outputStream().use {
            instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG,100,it)
        }
    }
}
