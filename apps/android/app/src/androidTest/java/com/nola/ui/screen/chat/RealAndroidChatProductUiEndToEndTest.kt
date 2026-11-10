package com.nola.ui.screen.chat

import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.printToString
import androidx.compose.ui.semantics.getOrNull
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import android.os.SystemClock
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.expertMessageSource
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX
import com.nola.ui.components.StreamingMarkdownPresentedTextKey
import java.io.File
import java.io.FileOutputStream
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RealAndroidChatProductUiEndToEndTest {
    @get:Rule
    val composeRule = createEmptyComposeRule()

    private fun awaitIsolatedWindow(scenario: ActivityScenario<MainActivity>) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        requireIsolatedProductTestUser(instrumentation.targetContext)
        // The owned disposable AVD is already unlocked; never dismiss a
        // keyguard or change OS security as part of UI acceptance.
        scenario.onActivity { activity ->
            activity.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        composeRule.waitUntil(15_000) {
            runCatching { composeRule.onAllNodesWithTag("navigation.bottom").fetchSemanticsNodes().isNotEmpty() }.getOrDefault(false)
        }
    }

    private fun confirmRealProviderCapabilities() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        composeRule.onNodeWithContentDescription(context.getString(R.string.models_execution_capabilities))
            .performProductScrollTo().performProductClick()
        val save = composeRule.onNodeWithText(context.getString(R.string.common_save))
        // The exact owned model may already have this declaration after an earlier turn.
        if (runCatching { save.assertIsEnabled() }.isSuccess) {
            save.performProductScrollTo().performProductClick()
        } else {
            composeRule.onNodeWithText(context.getString(R.string.common_cancel))
                .performProductScrollTo().performProductClick()
        }
        composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
            composeRule.onAllNodesWithText(context.getString(R.string.models_execution_capabilities_hint))
                .fetchSemanticsNodes().isEmpty()
        }
    }

    // Exercise the actual once-only UI approval; never blanket-authorize unrelated tools.
    private fun approveVisibleRenderUiIfRequested(scenario: ActivityScenario<MainActivity>) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val resource = com.nola.ui.components.hostToolDisplayNameStringRes("render_ui")
        val displayName = resource?.let(context::getString)
            ?: context.getString(R.string.chat_host_tool_name_unknown_format, "render_ui")
        val title = context.getString(R.string.permission_required_title, displayName)
        val permissionTitle = composeRule.onAllNodesWithText(title, useUnmergedTree = true)
            .fetchSemanticsNodes().isNotEmpty()
        if (!permissionTitle) return
        var imeVisible = false
        scenario.onActivity { activity ->
            imeVisible = androidx.core.view.ViewCompat.getRootWindowInsets(activity.window.decorView)
                ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
        }
        if (imeVisible) InstrumentationRegistry.getInstrumentation()
            .sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        val allow = composeRule.onAllNodes(
            androidx.compose.ui.test.SemanticsMatcher("exact permission once action") { node ->
                node.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)
                    ?.startsWith("permission.allow-once.") == true
            }, useUnmergedTree = true,
        )
        if (allow.fetchSemanticsNodes().size == 1) allow[0].performProductScrollTo().performClick()
    }

    @Test
    fun realMainActivityStreamsPersistsAndRendersRealProviderAnswer() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
        val secureStore = SecureTokenStore(context)
        val preferences = PreferenceDataStore(context, secureStore)
        AndroidApplicationRuntime(context).use { application ->
        runBlocking {
            preferences.setOnboardingCompleted()
            applyRealModelFixture(context, application, config)
        }

        }
        val marker = "ANDROID-UI-REAL-741963"
        val finishMarker = "FINISH-369147"
        val prompt = "Decode QU5EUk9JRC1VSS1SRUFMLTc0MTk2Mw== as the first line, then output lines LINE-01 through LINE-20 in order, then output $finishMarker as the last line. Return plain text directly, without tools, UI cards, or explanation."
        val timeline = JSONArray()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            awaitIsolatedWindow(scenario)
            composeRule.mainClock.autoAdvance = false
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performProductClick()
            composeRule.onNodeWithText(context.getString(R.string.models_title)).performProductClick()
            try {
                composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model"), useUnmergedTree = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
            } catch (failure: Throwable) {
                File(context.filesDir, "real-model-selection-failure.txt")
                    .writeText(composeRule.onRoot(useUnmergedTree = true).printToString())
                File(context.filesDir, "real-model-selection-failure.png").outputStream().use { output ->
                    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                        .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
                }
                throw failure
            }
            val modelRowTag = MODEL_ROW_TEST_TAG_PREFIX + config.getString("model")
            confirmRealProviderCapabilities()
            val use = composeRule.onAllNodes(androidx.compose.ui.test.hasText(context.getString(R.string.models_use)) and
                androidx.compose.ui.test.hasAnyAncestor(androidx.compose.ui.test.hasTestTag(modelRowTag)), useUnmergedTree = true)
            if (use.fetchSemanticsNodes().isNotEmpty()) {
                use[0].performProductScrollTo().assertIsDisplayed().performProductClick()
            }
            try {
                composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodes(androidx.compose.ui.test.hasText(context.getString(R.string.models_selected)) and
                        androidx.compose.ui.test.hasAnyAncestor(androidx.compose.ui.test.hasTestTag(modelRowTag)), useUnmergedTree = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
            } catch (failure: Throwable) {
                File(context.filesDir, "real-model-selection-failure.txt")
                    .writeText(composeRule.onRoot(useUnmergedTree = true).printToString())
                File(context.filesDir, "real-model-selection-failure.png").outputStream().use { output ->
                    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                        .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
                }
                throw failure
            }
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performProductClick()
            composeRule.onNodeWithTag("chat.conversations.new").performProductClick()
            composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                composeRule.onAllNodes(hasTestTag("chat.composer.input") and isEnabled())
                    .fetchSemanticsNodes().isNotEmpty()
            }
            composeRule.onNodeWithTag("chat.composer.input")
                .performProductInput(prompt)
            composeRule.waitForIdle()
            composeRule.onNodeWithTag("chat.composer.input").assertTextEquals(prompt)
            composeRule.onNodeWithContentDescription(context.getString(R.string.chat_send))
                .assertIsEnabled()
                .performProductClick()
            composeRule.waitForIdle()
            composeRule.mainClock.autoAdvance = false
            composeRule.waitUntil(15_000) {
                composeRule.mainClock.advanceTimeBy(32)
                composeRule.onNodeWithTag("chat-transcript-list").fetchSemanticsNode().config
                    .getOrNull(ChatTranscriptMessageIds)?.size == 2
            }
            val submittedIds = checkNotNull(composeRule.onNodeWithTag("chat-transcript-list")
                .fetchSemanticsNode().config.getOrNull(ChatTranscriptMessageIds))
            val expectedUserMessageId = submittedIds.first()
            val expectedAssistantMessageId = submittedIds.last()

            var previous = -1
            var previousThinking = -1
            var previousRunning: Boolean? = null
            val deadline = SystemClock.elapsedRealtime() + 180_000
            var finalSource = ""
            while (SystemClock.elapsedRealtime() < deadline) {
                approveVisibleRenderUiIfRequested(scenario)
                composeRule.mainClock.advanceTimeBy(64)
                composeRule.waitForIdle()
                val streamingNodes = composeRule
                    .onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                    .fetchSemanticsNodes()
                val presented = streamingNodes
                    .mapNotNull { it.config.getOrNull(StreamingMarkdownPresentedTextKey) }
                    .maxByOrNull(String::length)
                    .orEmpty()
                val source = streamingNodes
                    .mapNotNull { it.config.getOrNull(ChatStreamingMarkdownSourceKey) }
                    .maxByOrNull(String::length)
                    .orEmpty()
                if (source.length > finalSource.length) finalSource = source
                val characters = presented.length
                val running = composeRule.onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                    .fetchSemanticsNodes().any { it.config.getOrNull(ChatComposerGenerationActiveKey) == true }
                val visibleThinking = composeRule.onAllNodesWithTag("assistant.reasoning.block", useUnmergedTree = true)
                    .fetchSemanticsNodes().filter { it.boundsInRoot.width > 0f && it.boundsInRoot.height > 0f }
                    .maxOfOrNull { it.config.getOrNull(com.nola.ui.components.ThinkingBodyVisibleCharactersKey) ?: 0 } ?: 0
                if (characters != previous || visibleThinking != previousThinking || running != previousRunning) {
                    timeline.put(JSONObject().apply {
                        put("stage", "real-main-activity-turn")
                        put("running", running)
                        put("visibleThinkingCharacters", visibleThinking)
                        put("assistantCharacters", characters)
                        put("assistantText", presented)
                        put("sourceCharacters", source.length)
                        put("timestampMilliseconds", System.currentTimeMillis())
                        put("elapsedRealtimeMilliseconds", SystemClock.elapsedRealtime())
                    })
                    previous = characters; previousThinking = visibleThinking; previousRunning = running
                }
                val taskLinesPresent = (1..20).all { line -> "LINE-%02d".format(line) in source }
                val idleComposer = composeRule.onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                    .fetchSemanticsNodes().any { node -> node.boundsInRoot.width > 0f &&
                        node.config.getOrNull(ChatComposerGenerationActiveKey) == false }
                if (taskLinesPresent && idleComposer) break
                Thread.sleep(30)
            }
            composeRule.mainClock.advanceTimeBy(128)
            composeRule.waitForIdle()
            val composerDeadline = SystemClock.elapsedRealtime() + 10_000
            var composerIdle = false
            var composerStateEvidence = "[]"
            while (SystemClock.elapsedRealtime() < composerDeadline && !composerIdle) {
                composeRule.mainClock.advanceTimeBy(32)
                val composerNodes = composeRule
                    .onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                    .fetchSemanticsNodes()
                composerStateEvidence = composerNodes.joinToString(prefix = "[", postfix = "]") { node ->
                    "{bounds=${node.boundsInRoot},active=${node.config.getOrNull(ChatComposerGenerationActiveKey)}}"
                }
                composerIdle = composerNodes.any { node ->
                        node.boundsInRoot.width > 0f &&
                            node.boundsInRoot.height > 0f &&
                            node.config.getOrNull(ChatComposerGenerationActiveKey) == false
                }
                if (!composerIdle) Thread.sleep(50)
            }
            val tailDeadline = SystemClock.elapsedRealtime() + 10_000
            var tailVisible = false
            var tailStateEvidence = "missing"
            while (SystemClock.elapsedRealtime() < tailDeadline && !tailVisible) {
                composeRule.mainClock.advanceTimeBy(32)
                val tail = composeRule
                    .onAllNodesWithTag(CHAT_TRANSCRIPT_TAIL_TEST_TAG, useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .firstOrNull()
                val composer = composeRule
                    .onAllNodesWithTag("chat.composer.input", useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .firstOrNull()
                tailStateEvidence = "tail=${tail?.boundsInRoot},composer=${composer?.boundsInRoot}"
                tailVisible = tail != null && composer != null &&
                    tail.boundsInRoot.top >= 0f &&
                    tail.boundsInRoot.bottom <= composer.boundsInRoot.top - 8f
                if (!tailVisible) Thread.sleep(50)
            }
            composeRule.waitForIdle()
            val accessibility = composeRule.onRoot(useUnmergedTree = true).printToString(maxDepth = 100)
            File(context.filesDir, "real-android-ui-accessibility.txt").writeText(accessibility)
            File(context.filesDir, "real-android-ui-timeline.json").writeText(timeline.toString(2))
            FileOutputStream(File(context.filesDir, "real-android-ui.png")).use { output ->
                composeRule.onRoot().captureToImage().asAndroidBitmap()
                    .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
            }
            assertTrue(
                "real provider final markdown was not rendered: $finalSource",
                (1..20).all { line -> "LINE-%02d".format(line) in finalSource },
            )
            assertTrue(
                "visible composer did not project the completed turn: $composerStateEvidence",
                composerIdle,
            )
            assertTrue(
                "completed transcript tail is not visible above the composer: $tailStateEvidence",
                tailVisible,
            )
            val progressiveLengths = (0 until timeline.length())
                .map { timeline.getJSONObject(it).getInt("assistantCharacters") }
            val presentationMatchesCoreSource = (0 until timeline.length()).all { index ->
                val sample = timeline.getJSONObject(index)
                sample.getInt("assistantCharacters") == sample.getInt("sourceCharacters")
            }
            assertTrue(
                "Android replayed or lagged Core semantic batches: $timeline",
                presentationMatchesCoreSource && progressiveLengths.lastOrNull() == finalSource.length,
            )
            assertTrue(
                "rendered answer lost line structure or duplicated content: $finalSource",
                (1..20).all { line ->
                    val item = "LINE-%02d".format(line)
                    finalSource.lines().count { it.trim() == item } == 1
                },
            )
            val persistenceDeadline = SystemClock.elapsedRealtime() + 10_000
            var persisted = false
            var persistedTranscript: JSONArray? = null
            while (SystemClock.elapsedRealtime() < persistenceDeadline && !persisted) {
                val path = AndroidApplicationRuntime.defaultDatabaseFile(context)
                if (path.isFile) {
                    persisted = AndroidApplicationRuntime(context, path).use { runtime ->
                        runtime.listConversations().any { conversation ->
                            val messages = runtime.loadConversationProjectionV1(conversation.id).messages
                            val assistant = messages.singleOrNull { it.messageId == expectedAssistantMessageId }
                            val canonicalText = assistant?.parts
                                ?.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>()
                                ?.joinToString(separator = "") { it.markdown }
                            val hasAnswer = assistant?.role == com.nola.domain.projection.ConversationProjectionV1.Role.Assistant &&
                                assistant.state == com.nola.domain.projection.ConversationProjectionV1.MessageState.Completed &&
                                canonicalText == finalSource
                            if (hasAnswer) {
                                val window = runtime.loadConversationProjectionWindowV1(conversation.id, null, 80, 512_000)
                                val head = window.assistantJournalHeads.single { cursor ->
                                    cursor.getValue("identity").jsonObject.getValue("assistant_message_id").jsonPrimitive.content == expectedAssistantMessageId
                                }.getValue("identity").jsonObject
                                check(head.getValue("conversation_id").jsonPrimitive.content == conversation.id)
                                val runId = head.getValue("run_id").jsonPrimitive.content
                                val exact = runtime.executeQuery(buildJsonObject {
                                    put("type", "load_exact_run_invocation"); put("conversation_id", conversation.id)
                                    put("expected_run_id", runId); put("assistant_message_id", expectedAssistantMessageId)
                                }).jsonObject
                                check(exact.getValue("status").jsonPrimitive.content == "sealed")
                                val descriptor = com.nola.core.AndroidCanonicalTurnDescriptor(conversation.id, conversation.title,
                                    runId, expectedUserMessageId, prompt, null, assistantMessageId = expectedAssistantMessageId)
                                val seal = checkNotNull(com.nola.core.loadAndroidCanonicalTerminalSeal(descriptor,
                                    exact.getValue("core_session_id").jsonPrimitive.content, runtime::executeQuery))
                                check(seal.result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content == "answered")
                                persistedTranscript = JSONArray().apply {
                                    messages.forEach { message ->
                                        put(JSONObject().apply {
                                            put("role", message.role.name.lowercase())
                                            put("message_id", message.messageId); put("conversation_id", conversation.id)
                                            put("state", message.state.name)
                                            put(
                                                "text",
                                                message.parts
                                                    .filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>()
                                                    .joinToString(separator = "") { it.markdown },
                                            )
                                        })
                                    }
                                }
                            }
                            hasAnswer
                        }
                    }
                }
                if (!persisted) Thread.sleep(100)
            }
            assertTrue("rendered assistant answer was not durably persisted", persisted)
            val canonicalTranscript = requireNotNull(persistedTranscript) {
                "canonical transcript was not available after persistence"
            }
            assertTrue(
                "canonical transcript did not preserve user/assistant order: $canonicalTranscript",
                canonicalTranscript.length() == 2 &&
                    canonicalTranscript.getJSONObject(0).getString("role") == "user" &&
                    canonicalTranscript.getJSONObject(0).getString("text") == prompt &&
                    canonicalTranscript.getJSONObject(0).getString("message_id") == expectedUserMessageId &&
                    canonicalTranscript.getJSONObject(1).getString("role") == "assistant" &&
                    canonicalTranscript.getJSONObject(1).getString("message_id") == expectedAssistantMessageId &&
                    canonicalTranscript.getJSONObject(1).getString("state") == "Completed" &&
                    canonicalTranscript.getJSONObject(0).getString("conversation_id") == canonicalTranscript.getJSONObject(1).getString("conversation_id") &&
                    canonicalTranscript.getJSONObject(1).getString("text") == finalSource,
            )
            File(context.filesDir, "real-android-ui-transcript.json")
                .writeText(canonicalTranscript.toString(2))
            composeRule.waitForIdle()
            timeline.put(JSONObject().apply {
                put("stage", "real-main-activity-turn")
                put("running", false)
                val persistedSource = composeRule
                    .onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .mapNotNull { it.config.getOrNull(ChatStreamingMarkdownSourceKey) }
                    .maxByOrNull(String::length).orEmpty()
                put("assistantCharacters", persistedSource.length)
                put("assistantText", persistedSource)
                put("timestampMilliseconds", System.currentTimeMillis())
                        put("elapsedRealtimeMilliseconds", SystemClock.elapsedRealtime())
            })
            File(context.filesDir, "real-android-ui-timeline.json").writeText(timeline.toString(2))
            assertTrue("real provider answer was not rendered by MainActivity", accessibility.contains("LINE-20"))
        }
    }
    private fun androidx.compose.ui.test.SemanticsNodeInteraction.performProductScrollTo(): androidx.compose.ui.test.SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try { performScrollTo() } finally { composeRule.mainClock.autoAdvance = previous }
        composeRule.mainClock.advanceTimeBy(400)
        composeRule.waitForIdle()
        return this
    }

    private fun androidx.compose.ui.test.SemanticsNodeInteraction.performProductInput(text: String): androidx.compose.ui.test.SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try {
            performTextInput(text)
            composeRule.mainClock.advanceTimeBy(64)
            composeRule.waitForIdle()
        } finally { composeRule.mainClock.autoAdvance = previous }
        return this
    }

    private fun androidx.compose.ui.test.SemanticsNodeInteraction.performProductClick(): androidx.compose.ui.test.SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try {
            performClick()
            composeRule.mainClock.advanceTimeBy(400)
            composeRule.waitForIdle()
        } finally { composeRule.mainClock.autoAdvance = previous }
        return this
    }

    @Test
    fun realExpertReplyStreamsBeforeCompletionAndRestoresFromCanonicalStore() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
        val tokens = SecureTokenStore(context)
        runBlocking {
            PreferenceDataStore(context, tokens).setOnboardingCompleted()
            AndroidApplicationRuntime(context).use { runtime ->
                applyRealModelFixture(context, runtime, config)
            }
        }
        val expertName = "Review expert " + System.nanoTime().toString().takeLast(5)
        val startMarker = "EXPERT-STREAM-9173"
        val finishMarker = "EXPERT-FINISH-3719"
        val prompt = "Reply with $startMarker on the first line, then twenty numbered short lines about a fictional garden, then $finishMarker on the final line. No tools or additional commentary."
        val timeline = JSONArray()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            awaitIsolatedWindow(scenario)
            // Retained live owners may animate while a different page is open.
            // Drive real Compose frames explicitly rather than waiting for every
            // background animation in the process to become globally idle.
            composeRule.mainClock.autoAdvance = false
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performProductClick()
            composeRule.onNodeWithText(context.getString(R.string.models_title)).performProductClick()
            composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32); composeRule.onAllNodesWithTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
            if (composeRule.onAllNodesWithText(context.getString(R.string.models_use)).fetchSemanticsNodes().isNotEmpty()) {
                composeRule.onNodeWithText(context.getString(R.string.models_use)).performProductScrollTo().performProductClick()
            }
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performProductClick()
            composeRule.onNodeWithTag("chat.conversations.new").performProductClick()
            composeRule.onNodeWithTag("chat.topbar.more").performProductClick()
            composeRule.onNodeWithTag("chat.experts.open").performProductClick()
            composeRule.onNodeWithTag("chat.experts.manage").performProductClick()
            composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32); composeRule.onAllNodesWithTag("expert.create").fetchSemanticsNodes().any { !it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) } }
            composeRule.onNodeWithTag("expert.create").assertIsEnabled().performProductClick()
            composeRule.onNodeWithTag("expert.editor.name").performProductInput(expertName)
            composeRule.onNodeWithTag("expert.editor.description").performProductInput("Produces the requested garden list.")
            composeRule.onNodeWithTag("expert.editor.save").performProductClick()
            composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32); composeRule.onAllNodesWithTag("expert.editor.save").fetchSemanticsNodes().isEmpty() }
            composeRule.onNodeWithText(context.getString(R.string.common_back)).performProductClick()
            composeRule.mainClock.autoAdvance = true
            try { composeRule.onNodeWithTag("chat.experts.list").performScrollToNode(androidx.compose.ui.test.hasText(expertName)) }
            finally { composeRule.mainClock.autoAdvance = false }
            composeRule.onNodeWithText(expertName).performProductClick()
            composeRule.onNodeWithText(context.getString(R.string.common_done)).performProductClick()
            composeRule.onNodeWithTag("chat.composer.input").performProductInput(prompt)
            composeRule.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performProductClick()
            composeRule.mainClock.autoAdvance = false
            var sawLiveText = false
            var lastText = ""
            var replyTag = ""
            val deadline = SystemClock.elapsedRealtime() + 240_000
            while (SystemClock.elapsedRealtime() < deadline) {
                composeRule.mainClock.advanceTimeBy(64)
                approveVisibleRenderUiIfRequested(scenario)
                val rows = composeRule.onAllNodes(androidx.compose.ui.test.SemanticsMatcher.keyIsDefined(ExpertReplyStreamingKey), useUnmergedTree = true).fetchSemanticsNodes()
                val row = rows.singleOrNull()
                if (row != null) {
                    replyTag = row.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag).orEmpty()
                    val running = row.config.getOrNull(ExpertReplyStreamingKey) == true
                    val thinking = row.config.getOrNull(ExpertReplyThinkingLengthKey) ?: 0
                    val sources = composeRule.onAllNodes(androidx.compose.ui.test.hasAnyAncestor(androidx.compose.ui.test.hasTestTag(replyTag)) and androidx.compose.ui.test.hasTestTag(CHAT_STREAMING_MARKDOWN_TEST_TAG), useUnmergedTree = true)
                        .fetchSemanticsNodes().mapNotNull { it.config.getOrNull(ChatStreamingMarkdownSourceKey) }
                    val text = sources.maxByOrNull(String::length).orEmpty()
                    if (running && text.isNotBlank()) sawLiveText = true
                    if (text != lastText || timeline.length() == 0 ||
                        timeline.getJSONObject(timeline.length() - 1).getBoolean("running") != running ||
                        timeline.getJSONObject(timeline.length() - 1).getInt("thinkingCharacters") != thinking) timeline.put(JSONObject().apply {
                        put("running", running); put("characters", text.length); put("thinkingCharacters", thinking)
                        put("text", text); put("elapsedMs", SystemClock.elapsedRealtime())
                    })
                    lastText = text
                    if (!running && text.contains(startMarker) && "\n20." in text) break
                }
                Thread.sleep(25)
            }
            composeRule.mainClock.advanceTimeBy(128)
            composeRule.waitForIdle()
            File(context.filesDir, "real-android-expert-timeline.json").writeText(timeline.toString(2))
            File(context.filesDir, "real-android-expert-accessibility.txt").writeText(composeRule.onRoot(useUnmergedTree = true).printToString(maxDepth = 100))
            File(context.filesDir, "real-android-expert.png").outputStream().use { output ->
                InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
            }
            assertTrue("No real expert answer was visible before completion: $timeline", sawLiveText)
            assertTrue("Real expert task did not complete", lastText.contains(startMarker) && "\n20." in lastText)
            AndroidApplicationRuntime(context).use { runtime ->
                val matches = runtime.listConversations().flatMap { conversation ->
                    runtime.loadConversationProjectionV1(conversation.id).messages.mapNotNull { message ->
                        if (message.role != com.nola.domain.projection.ConversationProjectionV1.Role.User) null
                        else runtime.expertMessageSource(conversation.id, message.messageId)
                    }
                }.filter { manifest -> manifest.entries.any {
                    "expert.reply.${it.childSubmissionId}" == replyTag
                } }
                val manifest = matches.single()
                check(manifest.input.text == prompt)
                val entry = manifest.entries.single()
                check(replyTag == "expert.reply.${entry.childSubmissionId}")
                val receipt = checkNotNull(com.nola.data.repository.AgentRoomRepository(runtime).submission(manifest.input.roomId, entry.childSubmissionId))
                check(receipt.contextConversationId == entry.participant.contextConversationId && receipt.executionId == entry.childSubmissionId)
                check(receipt.state in setOf("completed", "completed_with_warnings"))
                val source = runtime.loadConversationProjectionV1(entry.participant.contextConversationId)
                val canonicalAssistant = source.messages.single { it.messageId == entry.childAssistantMessageId }
                check(canonicalAssistant.state == com.nola.domain.projection.ConversationProjectionV1.MessageState.Completed)
                val canonicalText = canonicalAssistant.parts
                    .filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>()
                    .joinToString("\n") { it.markdown }
                check(lastText == canonicalText) {
                    "Expert presentation omitted canonical terminal content: visible=${lastText.length}, canonical=${canonicalText.length}"
                }
            }
            scenario.recreate()
            composeRule.mainClock.advanceTimeBy(1_000)
            composeRule.waitUntil(20_000) { composeRule.mainClock.advanceTimeBy(32); composeRule.onAllNodesWithTag(replyTag).fetchSemanticsNodes().isNotEmpty() }
            composeRule.onAllNodes(androidx.compose.ui.test.hasText(expertName) and
                androidx.compose.ui.test.hasAnyAncestor(androidx.compose.ui.test.hasTestTag(replyTag)),
                useUnmergedTree = true).assertCountEquals(1)
        }
    }

    @Test
    fun realStreamingStopAllowsNextTurnAndActivityRestore() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val config = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
        val secureStore = SecureTokenStore(context)
        runBlocking {
            PreferenceDataStore(context, secureStore).setOnboardingCompleted()
            AndroidApplicationRuntime(context).use { runtime ->
                applyRealModelFixture(context, runtime, config)
            }
        }
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            awaitIsolatedWindow(scenario)
            composeRule.mainClock.autoAdvance = false
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performProductClick()
            composeRule.onNodeWithText(context.getString(R.string.models_title)).performProductClick()
            try {
                composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model"), useUnmergedTree = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
            } catch (failure: Throwable) {
                File(context.filesDir, "real-model-selection-failure.txt")
                    .writeText(composeRule.onRoot(useUnmergedTree = true).printToString())
                File(context.filesDir, "real-model-selection-failure.png").outputStream().use { output ->
                    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                        .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
                }
                throw failure
            }
            confirmRealProviderCapabilities()
            val useModel = composeRule.onAllNodesWithText(context.getString(R.string.models_use))
            if (useModel.fetchSemanticsNodes().isNotEmpty()) {
                composeRule.onNodeWithText(context.getString(R.string.models_use)).performProductScrollTo().performProductClick()
            }
            composeRule.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performProductClick()
            composeRule.onNodeWithTag("chat.conversations.new").performProductClick()
            composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                composeRule.onAllNodes(hasTestTag("chat.composer.input") and isEnabled())
                    .fetchSemanticsNodes().isNotEmpty()
            }
            composeRule.onNodeWithTag("chat.composer.input").performProductInput(
                "Analyze a practical plan for a small community observatory. Compare telescope types, weather and visibility risks, accessibility for beginners, observing schedules, maintenance, and realistic go/no-go criteria. Explain concrete tradeoffs and uncertainties in a coherent detailed report. Do not use tools.",
            )
            composeRule.onNodeWithContentDescription(context.getString(R.string.chat_send)).performProductClick()
            try {
                composeRule.waitUntil(180_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                        .fetchSemanticsNodes().any {
                            (it.config.getOrNull(ChatStreamingMarkdownSourceKey)?.length ?: 0) > 30
                        } || composeRule.onAllNodesWithTag("assistant.reasoning.block", useUnmergedTree = true)
                        .fetchSemanticsNodes().any {
                            it.boundsInRoot.width > 0f && it.boundsInRoot.height > 0f &&
                                (it.config.getOrNull(com.nola.ui.components.ThinkingBodyVisibleCharactersKey) ?: 0) > 30
                        }
                }
                // Exercise the real OS background/return path while this
                // provider-backed task is still active, before explicit Stop.
                val instrumentation = InstrumentationRegistry.getInstrumentation()
                // Launch Home from the target user's context. UiAutomation's
                // global Home action can target user 0 while this owned test
                // runs in a secondary user, leaving our Activity resumed.
                context.startActivity(android.content.Intent(android.content.Intent.ACTION_MAIN).apply {
                    addCategory(android.content.Intent.CATEGORY_HOME)
                    addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
                })
                composeRule.waitUntil(10_000) { scenario.state != androidx.lifecycle.Lifecycle.State.RESUMED }
                val backgroundState = scenario.state.name
                context.startActivity(android.content.Intent(context, MainActivity::class.java).apply {
                    addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK or android.content.Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
                })
                composeRule.waitUntil(15_000) {
                    composeRule.mainClock.advanceTimeBy(32)
                    scenario.state == androidx.lifecycle.Lifecycle.State.RESUMED &&
                        composeRule.onAllNodesWithContentDescription(context.getString(R.string.chat_stop))
                            .fetchSemanticsNodes().isNotEmpty()
                }
                File(context.filesDir, "real-android-stop-background.json").writeText(JSONObject()
                    .put("backgroundState", backgroundState).put("returnedWhileTaskActive", true).toString())
                composeRule.onNodeWithContentDescription(context.getString(R.string.chat_stop))
                    .assertIsEnabled().performProductClick()
                composeRule.waitUntil(30_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                        .fetchSemanticsNodes().any { it.config.getOrNull(ChatComposerGenerationActiveKey) == false }
                }
                composeRule.waitUntil(30_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodes(hasTestTag("chat.composer.input") and isEnabled())
                        .fetchSemanticsNodes().isNotEmpty()
                }
                composeRule.onNodeWithTag("chat.composer.input").performProductInput(
                    "Compute 37 times 19. Reply only with AFTER_STOP= followed by the result. Do not use tools.",
                )
                composeRule.waitUntil(30_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodes(androidx.compose.ui.test.hasContentDescription(
                        context.getString(R.string.chat_send)) and isEnabled()).fetchSemanticsNodes().isNotEmpty()
                }
                composeRule.onNodeWithContentDescription(context.getString(R.string.chat_send))
                    .assertIsEnabled().performProductClick()
                composeRule.waitUntil(180_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithText("AFTER_STOP=703", substring = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
                composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                        .fetchSemanticsNodes().any { it.config.getOrNull(ChatComposerGenerationActiveKey) == false }
                }
                scenario.recreate()
                composeRule.mainClock.advanceTimeBy(1_000)
                composeRule.waitUntil(15_000) { composeRule.mainClock.advanceTimeBy(32);
                    composeRule.onAllNodesWithText("AFTER_STOP=703", substring = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
            } finally {
                val instrumentation = InstrumentationRegistry.getInstrumentation()
                var imeVisible = false
                scenario.onActivity { activity -> imeVisible = androidx.core.view.ViewCompat.getRootWindowInsets(activity.window.decorView)
                    ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true }
                if (imeVisible) instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                composeRule.waitUntil(10_000) {
                    composeRule.mainClock.advanceTimeBy(32)
                    scenario.onActivity { activity -> imeVisible = androidx.core.view.ViewCompat.getRootWindowInsets(activity.window.decorView)
                        ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true }
                    !imeVisible
                }
                composeRule.mainClock.advanceTimeBy(1_000)
                instrumentation.waitForIdleSync()
                SystemClock.sleep(350)
                val composerBottom = composeRule.onNodeWithTag("chat.composer").fetchSemanticsNode().boundsInWindow.bottom
                var windowHeight = 0
                var navigationInset = 0
                scenario.onActivity { activity ->
                    windowHeight = activity.windowManager.currentWindowMetrics.bounds.height()
                    navigationInset = androidx.core.view.ViewCompat.getRootWindowInsets(activity.window.decorView)
                        ?.getInsets(androidx.core.view.WindowInsetsCompat.Type.systemBars())?.bottom ?: 0
                }
                File(context.filesDir, "real-android-stop-window-metrics.json").writeText(JSONObject().apply {
                    put("imeVisible", imeVisible); put("windowHeight", windowHeight); put("navigationInset", navigationInset)
                    put("composerBottom", composerBottom); put("composerBottomGap", windowHeight - navigationInset - composerBottom)
                }.toString(2))
                File(context.filesDir, "real-android-stop-accessibility.txt")
                    .writeText(composeRule.onRoot(useUnmergedTree = true).printToString(maxDepth = 100))
                File(context.filesDir, "real-android-stop.png").outputStream().use {
                    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                        .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
                }
            }
        }
    }

}
