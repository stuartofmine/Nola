// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.onboarding

import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.test.click
import androidx.compose.ui.test.performTouchInput
import android.graphics.Bitmap
import android.os.SystemClock
import java.io.File
import java.util.UUID
import android.view.KeyEvent
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.datastore.preferenceDataStore
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class FirstRunNavigationTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun chooseModelAndSkipPreservePreferencesAndOpenTheirRealDestinations() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val store = context.preferenceDataStore
        val completed = booleanPreferencesKey("has_completed_onboarding")
        val plannedSources = stringPreferencesKey("onboarding_planned_sources")
        val before = runBlocking { store.data.first() }
        fun waitForScreen(tag: String) {
            compose.waitUntil(15_000) { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag(tag).assertIsDisplayed()
        }
        val images = File(context.filesDir, "onboarding-images/${UUID.randomUUID()}")
        check(images.mkdirs())
        fun capture(name: String) {
            compose.waitForIdle()
            val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
            // Compose can be idle while WindowManager is still showing the old activity.
            automation.waitForIdle(400, 5_000)
            val deadline = SystemClock.elapsedRealtime() + 8_000
            fun captureVisibleApp(): Bitmap {
                do {
                    if (automation.rootInActiveWindow?.packageName == context.packageName) {
                        automation.takeScreenshot()?.let { return it }
                    }
                    SystemClock.sleep(100)
                } while (SystemClock.elapsedRealtime() < deadline)
                error("No screenshot of the Nola window at $name")
            }
            var image = captureVisibleApp()
            try {
                while (true) {
                    SystemClock.sleep(300)
                    val next = captureVisibleApp()
                    val settled = next.sameAs(image)
                    image.recycle()
                    image = next
                    if (settled) break
                    check(SystemClock.elapsedRealtime() < deadline) { "Screen was still changing at $name" }
                }
                File(images, "$name.png").outputStream().use { check(image.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { image.recycle() }
        }
        try {
            for (setup in listOf(true, false)) {
                runBlocking {
                    store.edit { it.remove(completed); it[plannedSources] = "calendar,documents" }
                }
                ActivityScenario.launch(MainActivity::class.java).use {
                    try {
                        val action = context.getString(if (setup) R.string.onboarding_choose_model else R.string.onboarding_guide_action_later)
                        compose.waitUntil(15_000) { compose.onAllNodesWithText(action).fetchSemanticsNodes().isNotEmpty() }
                        val path = if (setup) "choose-model" else "skip"
                        capture("$path-before")
                        compose.onNodeWithText(action).assertIsDisplayed().performTouchInput { click() }
                        if (setup) {
                            waitForScreen("models.refresh")
                            listOf(R.string.models_service_tab, R.string.models_local_models, R.string.models_web_host).forEach { title ->
                                val node = compose.onNodeWithText(context.getString(title), useUnmergedTree = true)
                                val layouts = mutableListOf<TextLayoutResult>()
                                val action = node.fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult]
                                check(action.action?.invoke(layouts) == true) { "No text layout for ${context.getString(title)}" }
                                val layout = layouts.single()
                                val last = layout.lineCount - 1
                                check((0..last).none(layout::isLineEllipsized) &&
                                    layout.getLineEnd(last, visibleEnd = true) == context.getString(title).length) {
                                    "Truncated model tab: ${context.getString(title)}"
                                }
                            }
                            capture("choose-model-after")
                            compose.onNodeWithText(context.getString(R.string.models_local_models)).performTouchInput { click() }
                            compose.waitUntil(5_000) { compose.onNodeWithText(context.getString(R.string.models_download_models), ignoreCase = true).isDisplayed() }
                            capture("local-models")
                            compose.onNodeWithText(context.getString(R.string.models_web_host)).performTouchInput { click() }
                            compose.waitUntil(5_000) { compose.onNodeWithText(context.getString(R.string.models_desktop_bridge)).isDisplayed() }
                            capture("host-connections")
                            compose.onNodeWithText(context.getString(R.string.models_service_tab)).performTouchInput { click() }
                            compose.waitUntil(15_000) {
                                compose.onAllNodesWithTag("model.setup.endpoint").fetchSemanticsNodes().isNotEmpty() ||
                                    compose.onAllNodesWithTag("models.connection.api").fetchSemanticsNodes().isNotEmpty()
                            }
                            if (compose.onAllNodesWithTag("model.setup.endpoint").fetchSemanticsNodes().isEmpty()) {
                                compose.onNodeWithTag("models.connection.api").performTouchInput { click() }
                            }
                            waitForScreen("model.setup.endpoint")
                            capture("service-address")
                            InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
                        }
                        waitForScreen("chat.conversations.list")
                        capture("$path-chat")
                    } catch (failure: Throwable) {
                        runCatching { capture("failure") }.onFailure(failure::addSuppressed)
                        throw failure
                    }
                }
                ActivityScenario.launch(MainActivity::class.java).use {
                    waitForScreen("chat.conversations.list")
                    capture(if (setup) "choose-model-reopened" else "skip-reopened")
                }
                val stored = runBlocking { store.data.first() }
                assertEquals(true, stored[completed])
                assertEquals("calendar,documents", stored[plannedSources])
            }
        } finally {
            runBlocking {
                store.edit { preferences ->
                    before[completed]?.let { preferences[completed] = it } ?: preferences.remove(completed)
                    before[plannedSources]?.let { preferences[plannedSources] = it } ?: preferences.remove(plannedSources)
                }
            }
        }
    }
}
