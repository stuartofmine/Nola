// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.ui.assertWarmThemeSurface
import com.nola.ui.colorsMatchWithinTolerance
import com.nola.ui.dominantInteriorColor
import com.nola.ui.toHexRgb
import com.nola.ui.wcagContrastRatio
import com.nola.ui.screen.onboarding.OnboardingContent
import com.nola.ui.screen.onboarding.OnboardingUiState
import com.nola.ui.theme.NolaTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult
import java.io.File
import kotlin.math.roundToInt
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
        val destination = (hasText(context.getString(label)) or hasContentDescription(context.getString(label))) and hasAnyAncestor(primaryNavigation)
        compose.onNode(destination).performClick()
        compose.onNode(destination and isSelected()).assertIsDisplayed()
    }
    private fun capture(name: String, darkTheme: Boolean? = null) {
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
        // Capture the frame shown by the emulator's compositor. Compose's window
        // PixelCopy can time out on an idle page with no new buffers to submit.
        val root = compose.onRoot().fetchSemanticsNode()
        val (position, size) = compose.runOnIdle { root.positionOnScreen to root.size }
        val screen = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()) {
            "The compositor did not return a screenshot for $name"
        }
        val cropped = Bitmap.createBitmap(screen, position.x.roundToInt(), position.y.roundToInt(), size.width, size.height)
        if (cropped !== screen) screen.recycle()
        val image = cropped.asImageBitmap()
        if (darkTheme != null) {
            val pixels = image.assertWarmThemeSurface(darkTheme = darkTheme, label = "WorkspacePageMatrix($name)")
            val surfaceSample = pixels.dominantInteriorColor()
            val textNodes = compose.onAllNodes(
                SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult),
                useUnmergedTree = true,
            ).fetchSemanticsNodes()
            val results = mutableListOf<TextLayoutResult>()
            for (node in textNodes) {
                results.clear()
                node.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(results)
                for (layout in results) {
                    val color = layout.layoutInput.style.color
                    if (color == Color.Unspecified || color.alpha < 0.9f) continue
                    if (darkTheme) {
                        org.junit.Assert.assertFalse(
                            "WorkspacePageMatrix($name) rendered near-black text ${color.toHexRgb()} in Dark mode",
                            colorsMatchWithinTolerance(color, Color.Black, 10),
                        )
                    }
                    if (colorsMatchWithinTolerance(color, if (darkTheme) Color(0xFFF2F0EB) else Color(0xFF181817), 8) ||
                        colorsMatchWithinTolerance(color, if (darkTheme) Color(0xFFA3A09A) else Color(0xFF5C5B57), 8)
                    ) {
                        val ratio = wcagContrastRatio(color, surfaceSample)
                        org.junit.Assert.assertTrue(
                            "WorkspacePageMatrix($name) text ${color.toHexRgb()} vs ${surfaceSample.toHexRgb()} contrast $ratio < 4.5",
                            ratio >= 4.5f,
                        )
                    }
                }
            }
        }
        val directory = File(context.filesDir, "page-matrix").apply { mkdirs() }
        File(directory, "$name.png").outputStream().use {
            check(image.asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it))
        }
        cropped.recycle()
    }
    private fun back(activity: ActivityScenario<MainActivity>) {
        compose.waitForIdle()
        activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
    }

    private fun enterWorkspace() {
        val skip = hasText(context.getString(R.string.onboarding_guide_action_later))
        compose.waitUntil(15_000) {
            compose.onAllNodes(primaryNavigation or skip).fetchSemanticsNodes().isNotEmpty()
        }
        if (compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isEmpty()) {
            compose.onNode(skip).performClick()
        }
        compose.waitUntil(15_000) { compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun allPrimaryAndManagementPagesOpenAndReturnInBothThemes() {
        requireIsolatedProductTestUser(context)
        // Only this test's prior screenshots; never clear application records.
        File(context.filesDir, "page-matrix").deleteRecursively()
        // Verify OnboardingScreen renders Warm Paper & Warm Ink surfaces across both themes
        // without mutating product onboarding persistence.
        for ((theme, isDark) in listOf("light" to false, "dark" to true)) {
            ActivityScenario.launch(ComponentActivity::class.java).use { host ->
                host.onActivity { activity ->
                    activity.setContent {
                        NolaTheme(darkTheme = isDark) {
                            OnboardingContent(uiState = OnboardingUiState(), onComplete = {})
                        }
                    }
                }
                compose.waitForIdle()
                compose.onNodeWithText(context.getString(R.string.onboarding_choose_model)).assertIsDisplayed()
                compose.onNodeWithText(context.getString(R.string.onboarding_guide_action_later)).assertIsDisplayed()
                capture("onboarding-$theme", darkTheme = isDark)
            }
        }
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            for ((theme, label) in listOf("light" to R.string.appearance_light, "dark" to R.string.appearance_dark)) {
                val isDark = theme == "dark"
                tab(R.string.tab_settings)
                compose.onNodeWithText(context.getString(R.string.settings_general)).performClick()
                compose.onNodeWithText(context.getString(label)).performClick()
                compose.waitUntil(10_000) {
                    compose.onAllNodes(hasText(context.getString(label)) and isSelected()).fetchSemanticsNodes().isNotEmpty()
                }
                compose.mainClock.advanceTimeBy(1_000)
                compose.waitForIdle()
                capture("settings-appearance-$theme", darkTheme = isDark)
                back(activity)
                for (section in listOf(R.string.settings_general, R.string.settings_privacy, R.string.models_title,
                    R.string.settings_capabilities)) {
                    tab(R.string.tab_settings)
                    compose.onNodeWithText(context.getString(section)).performScrollTo().performClick()
                    capture("settings-${context.resources.getResourceEntryName(section)}-$theme", darkTheme = isDark)
                    if (section == R.string.settings_general) {
                        compose.onNodeWithTag("settings_notification_center").performScrollTo().performClick()
                        back(activity)
                        capture("general-return-$theme", darkTheme = isDark)
                        compose.onNodeWithTag("settings_notification_center").assertIsDisplayed()
                        compose.onNodeWithText(context.getString(R.string.software_update_check)).performScrollTo().assertIsDisplayed()
                        capture("general-about-$theme", darkTheme = isDark)
                    }
                    back(activity)
                    compose.onNodeWithText(context.getString(R.string.settings_general)).assertIsDisplayed()
                }
                tab(R.string.tab_information)
                capture("information-$theme", darkTheme = isDark)
                val overview = compose.onAllNodesWithTag("information.overview").fetchSemanticsNodes().isNotEmpty()
                for (page in if (overview) listOf(4, 1, 0) else listOf(4, 1, 0, 2, 3)) {
                    compose.onNodeWithTag(if (overview) "information.category.$page" else "workspace.section.$page")
                        .performScrollTo().performClick()
                    capture("information-$page-$theme", darkTheme = isDark)
                    if (overview) {
                        back(activity)
                        compose.onNodeWithTag("information.overview").assertIsDisplayed()
                    }
                }
                for (destination in listOf(R.string.tab_chat, R.string.tab_activity, R.string.tab_settings)) {
                    tab(destination)
                    if (destination == R.string.tab_activity) {
                        val layouts = mutableListOf<TextLayoutResult>()
                        val label = hasText(context.getString(R.string.activity_search_title)) and
                            SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult)
                        compose.onNode(label, useUnmergedTree = true)
                            .performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
                        val layout = layouts.single()
                        org.junit.Assert.assertFalse("Activity search label must fit beside the button", layout.isLineEllipsized(0))
                    }
                    capture("primary-${context.resources.getResourceEntryName(destination)}-$theme", darkTheme = isDark)
                }
            }
        }
    }

    @Test fun generalScrollSurvivesNotificationCenterRoundTrip() {
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            tab(R.string.tab_settings)
            compose.onNodeWithText(context.getString(R.string.settings_general)).performClick()
            compose.onNodeWithText(context.getString(R.string.software_update_check)).performScrollTo()
            val center = compose.onNodeWithTag("settings_notification_center")
            center.performScrollTo()
            val scroller = compose.onNode(hasScrollAction())
            fun range() = scroller.fetchSemanticsNode()
                .config[androidx.compose.ui.semantics.SemanticsProperties.VerticalScrollAxisRange]
            // A tall tablet may show every setting at once. If it can scroll,
            // always exercise a nonzero offset without moving the action off-screen.
            if (range().maxValue() > 0f && range().value() == 0f) {
                scroller.performSemanticsAction(SemanticsActions.ScrollBy) { it(0f, 12f) }
            }
            val before = range().value()
            org.junit.Assert.assertTrue("Scrollable General must exercise a reading position", range().maxValue() == 0f || before > 0f)
            center.performClick()
            back(activity)
            center.assertIsDisplayed()
            org.junit.Assert.assertEquals("Returning from notifications preserves the reading position", before, range().value(), 1f)
        }
    }

}
