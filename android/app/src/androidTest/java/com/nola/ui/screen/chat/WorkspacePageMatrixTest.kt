// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** MainActivity navigation at phone, compact-side and wide-side sizes in a disposable user. No inference. */
@RunWith(AndroidJUnit4::class)
class WorkspacePageMatrixTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val primaryNavigation = hasTestTag("navigation.bottom") or hasTestTag("navigation.side.compact") or hasTestTag("navigation.side.expanded")
    private fun tab(label: Int) {
        compose.onNode((hasText(context.getString(label)) or hasContentDescription(context.getString(label))) and hasAnyAncestor(primaryNavigation)).performClick()
        compose.waitForIdle()
    }
    private fun capture(name: String) {
        compose.mainClock.advanceTimeBy(500)
        compose.waitForIdle()
        // Android's native ripple runs on RenderThread, outside Compose's
        // virtual clock. Capture the settled page rather than a recent tap.
        android.os.SystemClock.sleep(400)
        compose.waitForIdle()
        if (compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty()) {
            compose.onAllNodes(isSelected() and hasAnyAncestor(primaryNavigation))
                .assertCountEquals(1)
        }
        val directory = File(context.filesDir, "page-matrix").apply { mkdirs() }
        File(directory, "$name.png").outputStream().use {
            check(compose.onRoot().captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it))
        }
    }
    private fun back(activity: ActivityScenario<MainActivity>) {
        activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
    }
    @Test fun allPrimaryAndManagementPagesOpenAndReturnInBothThemes() {
        requireIsolatedProductTestUser(context)
        // Only this test's prior screenshots; never clear application records.
        File(context.filesDir, "page-matrix").deleteRecursively()
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            compose.waitUntil(15_000) {
                compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty()
                    || compose.onAllNodesWithText(context.getString(R.string.onboarding_guide_action_next)).fetchSemanticsNodes().isNotEmpty()
            }
            // The guide teaches real setup without pretend runtime/source selections
            // or requests for system permissions and model credentials.
            if (compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isEmpty()) {
                for (step in 0..3) {
                    for (role in listOf(androidx.compose.ui.semantics.Role.RadioButton, androidx.compose.ui.semantics.Role.Checkbox)) {
                        compose.onAllNodes(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.Role, role)).assertCountEquals(0)
                    }
                    capture("onboarding-$step")
                    val label = if (step == 3) R.string.onboarding_guide_action_start else R.string.onboarding_guide_action_next
                    compose.onNodeWithText(context.getString(label)).performClick()
                    compose.waitForIdle()
                }
            }
            compose.waitUntil(15_000) { compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty() }
            for ((theme, label) in listOf("light" to R.string.appearance_light, "dark" to R.string.appearance_dark)) {
                tab(R.string.tab_settings)
                compose.onNodeWithText(context.getString(R.string.appearance_title)).performClick()
                compose.onNodeWithText(context.getString(label)).performClick()
                compose.waitUntil(10_000) {
                    compose.onAllNodes(hasText(context.getString(label)) and isSelected()).fetchSemanticsNodes().isNotEmpty()
                }
                compose.mainClock.advanceTimeBy(1_000)
                compose.waitForIdle()
                capture("settings-appearance-$theme")
                back(activity)
                for (section in listOf(R.string.notifications_title, R.string.settings_privacy, R.string.models_title,
                    R.string.settings_capabilities, R.string.settings_data_sources, R.string.scheduled_title)) {
                    tab(R.string.tab_settings)
                    compose.onNodeWithText(context.getString(section)).performScrollTo().performClick()
                    capture("settings-$section-$theme")
                    back(activity)
                    compose.onNodeWithText(context.getString(R.string.appearance_title)).assertIsDisplayed()
                }
                tab(R.string.tab_information)
                capture("information-$theme")
                val overview = compose.onAllNodesWithTag("information.overview").fetchSemanticsNodes().isNotEmpty()
                for (page in if (overview) listOf(4, 1, 0) else listOf(4, 1, 0, 2, 3)) {
                    compose.onNodeWithTag(if (overview) "information.category.$page" else "workspace.section.$page")
                        .performScrollTo().performClick()
                    capture("information-$page-$theme")
                    if (overview) {
                        back(activity)
                        compose.onNodeWithTag("information.overview").assertIsDisplayed()
                    }
                }
                for (destination in listOf(R.string.tab_chat, R.string.tab_activity, R.string.tab_settings)) {
                    tab(destination)
                    capture("primary-$destination-$theme")
                }
            }
        }
    }
}
