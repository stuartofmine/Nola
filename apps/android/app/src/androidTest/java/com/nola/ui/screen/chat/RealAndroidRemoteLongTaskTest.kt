// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
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
    private fun screenshot(name: String) {
        File(context.filesDir, name).outputStream().use {
            InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }
    private fun approveExactJob() {
        val once = compose.onAllNodesWithText(context.getString(R.string.permission_action_allow_once))
        if (once.fetchSemanticsNodes().isNotEmpty()) {
            compose.onAllNodesWithText(context.getString(R.string.canonical_details)).onLast().performClick()
            compose.waitUntil(5000) {
                compose.onAllNodes(hasText("python3 remote_job.py", substring = true) and hasAnyAncestor(isDialog())).fetchSemanticsNodes().isNotEmpty()
            }
            val review = compose.onAllNodes(hasText("python3 remote_job.py", substring = true) and hasAnyAncestor(isDialog()))
                .onFirst().fetchSemanticsNode().config[SemanticsProperties.Text].joinToString("\n") { it.text }
            assertTrue("Only the exact owned job can be approved", review.lines().contains("Command: python3 remote_job.py"))
            assertTrue("Job must run in its isolated project", review.lines().any {
                it == "Working directory: Project directory" || it == "Working directory: " + config().getString("project")
            })
            screenshot("remote-long-permission-review.png")
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
            compose.onNodeWithTag("chat.composer.input").performTextInput(fixture.getString("prompt"))
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().performClick()
            compose.waitUntil(180_000) { approveExactJob(); state().optInt("ticks") >= 2 }
            val running = state()
            assertEquals("running", running.getString("runStatus"))
            assertEquals(1, running.getInt("starts"))
            screenshot("remote-long-running.png")
            File(context.filesDir, "remote-long-original.json").writeText(running.toString())
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
                screenshot("remote-long-restored-result.png")
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
