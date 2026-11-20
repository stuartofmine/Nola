// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.first
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Production UI + encrypted Host + actual model; oracle only reads owned Host state. */
@RunWith(AndroidJUnit4::class)
class RealAndroidRemoteLongTaskTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun config() = JSONObject(File(context.filesDir, "remote-long-fixture.json").readText())
    private fun state(): JSONObject {
        val connection = URL(config().getString("oracle")).openConnection() as HttpURLConnection
        connection.connectTimeout = 3000; connection.readTimeout = 3000
        return try { JSONObject(connection.inputStream.bufferedReader().use { it.readText() }) }
        finally { connection.disconnect() }
    }
    private fun screenshot(name: String, waitForTransition: Boolean = false) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        if (waitForTransition) {
            // Dialog windows animate on the platform compositor after Compose is idle.
            var previous: IntArray? = null
            var stableFrames = 0
            compose.waitUntil(5000) {
                android.os.SystemClock.sleep(100)
                val frame = automation.takeScreenshot()
                val preview = Bitmap.createScaledBitmap(frame, 40, 60, true)
                val pixels = IntArray(40 * 48)
                preview.getPixels(pixels, 0, 40, 0, 6, 40, 48)
                preview.recycle()
                frame.recycle()
                val prior = previous
                stableFrames = if (prior != null && pixels.indices.count { pixels[it] != prior[it] } <= 4) stableFrames + 1 else 0
                previous = pixels
                stableFrames >= 3
            }
        }
        val image = automation.takeScreenshot()
        try {
            File(context.filesDir, name).outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        } finally {
            image.recycle()
        }
    }
    private fun assertHeaderTextFits(text: String) {
        val label = hasText(text) and hasAnyAncestor(hasTestTag("chat.topbar.title"))
        var measured = "no layout"
        try {
            compose.waitUntil(5000) {
                val layouts = mutableListOf<TextLayoutResult>()
                compose.onNode(label, useUnmergedTree = true)
                    .performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
                val layout = layouts.single()
                val last = layout.lineCount - 1
                val end = layout.getLineEnd(last, visibleEnd = true)
                val ellipsized = (0..last).any { layout.isLineEllipsized(it) }
                val clipped = (0..last).any { layout.getLineLeft(it) < -1f || layout.getLineRight(it) > layout.size.width + 1f }
                measured = "size=${layout.size}, visibleEnd=$end, textLength=${text.length}, ellipsis=$ellipsized, clipped=$clipped"
                !ellipsized && !clipped && end == text.length
            }
        } catch (error: androidx.compose.ui.test.ComposeTimeoutException) {
            throw AssertionError("Clipped conversation header '$text': $measured", error)
        }
    }
    private fun approveExactJob() {
        val once = compose.onAllNodesWithText(context.getString(R.string.permission_action_allow_once))
        if (once.fetchSemanticsNodes().isNotEmpty()) {
            val allow = once.onFirst()
            val identity = allow.fetchSemanticsNode().config[SemanticsProperties.TestTag].removePrefix("permission.allow-once.")
            assertHeaderTextFits(context.getString(R.string.conversations_waiting))
            compose.onNodeWithTag("permission.details.$identity").performTouchInput { click() }
            compose.waitUntil(5000) {
                compose.onAllNodes(hasText("python3 remote_job.py", substring = true) and hasAnyAncestor(isDialog())).fetchSemanticsNodes().isNotEmpty()
            }
            val review = compose.onAllNodes(hasText("python3 remote_job.py", substring = true) and hasAnyAncestor(isDialog()))
                .onFirst().fetchSemanticsNode().config[SemanticsProperties.Text].joinToString("\n") { it.text }
            assertTrue("Only the exact owned job can be approved", review.lines().contains("Command: python3 remote_job.py"))
            assertTrue("Job must run in its isolated project", review.lines().any {
                it == "Working directory: Project directory" || it == "Working directory: " + config().getString("project")
            })
            compose.waitForIdle()
            screenshot("remote-long-permission-review.png", waitForTransition = true)
            compose.onNode(hasText(context.getString(R.string.common_close)) and hasAnyAncestor(isDialog())).performClick()
            once.onFirst().performClick()
        }
    }

    @Test fun submitRealLongTaskThenDetachClient() {
        requireIsolatedProductTestUser(context)
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("realRemoteLong") == "1")
        val fixture = config()
        val preferences = PreferenceDataStore(context, SecureTokenStore(context))
        runBlocking {
            preferences.setOnboardingCompleted()
            preferences.saveWebHostConfig(fixture.getString("url"), fixture.getString("token"), verifiedHostId = fixture.getString("hostId"))
        }
        ActivityScenario.launch(MainActivity::class.java).use {
            try {
                compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
                compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
                compose.onNode(hasText(context.getString(R.string.models_web_host)) and hasClickAction()).performClick()
                val hostRow = hasTestTag("models.row.web-host")
                compose.waitUntil(60_000) { compose.onAllNodes(hostRow).fetchSemanticsNodes().isNotEmpty() }
                val use = hasText(context.getString(R.string.models_use)) and hasAnyAncestor(hostRow)
                if (compose.onAllNodes(use).fetchSemanticsNodes().isNotEmpty()) compose.onNode(use).performScrollTo().performClick()
                compose.waitUntil(30_000) { compose.onAllNodes(hasText(context.getString(R.string.models_selected)) and hasAnyAncestor(hostRow)).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                compose.onNodeWithTag("chat.conversations.new").performClick()
                val readyComposer = hasTestTag("chat.composer.input") and isEnabled()
                compose.waitUntil(30_000) { compose.onAllNodes(readyComposer).fetchSemanticsNodes().isNotEmpty() }
                assertHeaderTextFits(context.getString(R.string.chat_new_chat))
                screenshot("remote-long-new-chat.png")
                val composer = compose.onNode(readyComposer)
                composer.performTouchInput { click() }
                composer.performTextInput(fixture.getString("prompt"))
                compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performClick()
                compose.waitUntil(180_000) { approveExactJob(); state().optInt("ticks") >= 2 }
                val running = state()
                assertEquals("running", running.getString("runStatus"))
                assertEquals(1, running.getInt("starts"))
                screenshot("remote-long-running.png")
                File(context.filesDir, "remote-long-original.json").writeText(running.toString())
            } catch (failure: Throwable) {
                runCatching {
                    screenshot("remote-long-failed-in-app.png")
                    File(context.filesDir, "remote-long-failed-tree.txt").writeText(
                        compose.onAllNodes(isRoot(), useUnmergedTree = true).printToString()
                    )
                }.onFailure(failure::addSuppressed)
                throw failure
            }
        }
        val before = state()
        Thread.sleep(5000)
        val detached = state()
        assertEquals(before.getString("runId"), detached.getString("runId"))
        assertEquals("running", detached.getString("runStatus"))
        assertTrue(detached.getInt("ticks") > before.getInt("ticks"))
        assertEquals(1, detached.getInt("starts"))
        File(context.filesDir, "remote-long-detached.json").writeText(detached.toString())
    }

    @Test fun coldClientObservesSameRunAndCompletedResult() {
        requireIsolatedProductTestUser(context)
        org.junit.Assume.assumeTrue(InstrumentationRegistry.getArguments().getString("realRemoteLong") == "1")
        val original = JSONObject(File(context.filesDir, "remote-long-original.json").readText())
        val conversation = original.getString("conversationId")
        ActivityScenario.launch(MainActivity::class.java).use {
            val tag = "chat.conversation.$conversation"
            compose.waitUntil(60_000) {
                compose.onAllNodesWithTag("chat.topbar.more").fetchSemanticsNodes().isNotEmpty() ||
                    compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty()
            }
            if (compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty())
                compose.onNodeWithTag(tag).performScrollTo().performClick()
            compose.waitUntil(240_000) {
                val observed = state()
                assertEquals(original.getString("runId"), observed.getString("runId"))
                assertEquals(1, observed.getInt("runCount"))
                observed.optString("outcome") == "answered"
            }
            val final = state()
            assertEquals(1, final.getInt("starts"))
            assertEquals(config().getInt("steps"), final.getInt("ticks"))
            assertTrue(final.getBoolean("resultValid"))
            val messageTag = CHAT_MESSAGE_TEST_TAG_PREFIX + original.getString("assistantMessageId")
            try {
                compose.waitUntil(45_000) {
                    runCatching { compose.onNodeWithTag("chat-transcript-list").performScrollToNode(hasTestTag(messageTag)) }
                    compose.onAllNodes(hasText("ANDROID_REMOTE_LONG_DONE", substring = true) and hasAnyAncestor(hasTestTag(messageTag)), useUnmergedTree = true)
                        .fetchSemanticsNodes().isNotEmpty()
                }
                compose.waitUntil(45_000) {
                    compose.onAllNodesWithContentDescription(context.getString(R.string.chat_stop)).fetchSemanticsNodes().isEmpty() &&
                        compose.onAllNodesWithContentDescription(context.getString(R.string.chat_send)).fetchSemanticsNodes().isNotEmpty()
                }
            } finally {
                compose.waitForIdle()
                screenshot("remote-long-restored-result.png", waitForTransition = true)
                File(context.filesDir, "remote-long-restored-tree.txt").writeText(compose.onRoot(useUnmergedTree = true).printToString())
            }
            File(context.filesDir, "remote-long-result.json").writeText(final.toString())
        }
    }
    @Test fun clearOwnedHostFixture() = runBlocking {
        requireIsolatedProductTestUser(context)
        val file = File(context.filesDir, "remote-long-fixture.json")
        if (file.isFile) {
            val fixture = JSONObject(file.readText())
            val preferences = PreferenceDataStore(context, SecureTokenStore(context))
            if (preferences.webHostBaseUrl.first() == fixture.getString("url")) preferences.clearWebHostConfig()
            file.delete()
        }
    }
}
