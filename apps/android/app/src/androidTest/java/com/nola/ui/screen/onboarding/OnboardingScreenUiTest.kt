// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.onboarding

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.ui.WarmPaletteExpectations
import com.nola.ui.assertFullyInside
import com.nola.ui.assertNonBlackDarkText
import com.nola.ui.assertRenderedBackgroundMatches
import com.nola.ui.assertRenderedTextColor
import com.nola.ui.theme.NolaTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class OnboardingScreenUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun allFourStepsAndCompletionFailureBannerRenderUnclippedAcrossFormFactorsAndThemes() {
        var widthDp by mutableStateOf(360)
        var heightDp by mutableStateOf(640)
        var fontScale by mutableStateOf(1f)
        var darkTheme by mutableStateOf(false)
        var completionFailed by mutableStateOf(false)

        compose.setContent {
            val baseDensity = LocalDensity.current
            CompositionLocalProvider(
                LocalDensity provides Density(baseDensity.density, fontScale),
            ) {
                NolaTheme(darkTheme = darkTheme) {
                    Box(
                        modifier = Modifier
                            .width(widthDp.dp)
                            .height(heightDp.dp)
                            .testTag("onboarding.viewport"),
                    ) {
                        OnboardingContent(
                            uiState = OnboardingUiState(completionFailed = completionFailed),
                            onComplete = {},
                        )
                    }
                }
            }
        }

        val laterLabel = compose.activity.getString(R.string.onboarding_guide_action_later)
        val nextLabel = compose.activity.getString(R.string.onboarding_guide_action_next)
        val startLabel = compose.activity.getString(R.string.onboarding_guide_action_start)
        val backLabel = compose.activity.getString(R.string.onboarding_guide_back)
        val errorLabel = compose.activity.getString(R.string.onboarding_guide_save_error)

        val stepTitles = listOf(
            compose.activity.getString(R.string.onboarding_core_loop_title),
            compose.activity.getString(R.string.onboarding_guide_runtime_title),
            compose.activity.getString(R.string.onboarding_guide_sources_title),
            compose.activity.getString(R.string.onboarding_guide_ready_title),
        )

        val configurations = listOf(
            Triple(360, 640, 1.0f),
            Triple(390, 844, 1.0f),
            Triple(1280, 800, 1.0f),
            Triple(360, 640, 1.5f),
        )

        for ((w, h, scale) in configurations) {
            for (isDark in listOf(false, true)) {
                compose.runOnIdle {
                    widthDp = w
                    heightDp = h
                    fontScale = scale
                    darkTheme = isDark
                    completionFailed = true
                }
                compose.waitForIdle()

                val rootBounds = compose.onNodeWithTag("onboarding.viewport").fetchSemanticsNode().boundsInRoot
                val expectedBg = if (isDark) WarmPaletteExpectations.DarkBackground else WarmPaletteExpectations.LightBackground
                compose.onNodeWithTag("onboarding.viewport")
                    .assertRenderedBackgroundMatches(expectedBg, isDark, "Onboarding(${w}x${h},scale=$scale,dark=$isDark)")

                for (stepIndex in 0..3) {
                    compose.waitForIdle()
                    compose.onNodeWithText(stepTitles[stepIndex]).assertIsDisplayed()
                    if (isDark) {
                        compose.onNodeWithText(stepTitles[stepIndex], useUnmergedTree = true)
                            .assertRenderedTextColor(WarmPaletteExpectations.DarkText, "OnboardingStepTitle($stepIndex)")
                        compose.onNodeWithText(laterLabel, useUnmergedTree = true)
                            .assertNonBlackDarkText("OnboardingLater($stepIndex)")
                    }
                    compose.onNodeWithText(laterLabel).assertIsDisplayed().assertFullyInside(rootBounds, "Later($w,$h)")
                    compose.onNodeWithText(errorLabel).assertIsDisplayed().assertFullyInside(rootBounds, "Error($w,$h)")

                    val actionLabel = if (stepIndex == 3) startLabel else nextLabel
                    compose.onNodeWithText(actionLabel).assertIsDisplayed().assertFullyInside(rootBounds, "Primary($w,$h,$stepIndex)")
                    if (stepIndex < 3) {
                        compose.onNodeWithText(actionLabel).performClick()
                    }
                }

                // Step back to step 0 before the next configuration pass.
                repeat(3) {
                    compose.onNodeWithText(stepTitles[3 - it]).assertExists()
                    compose.onNodeWithContentDescriptionCompat(backLabel).performClick()
                    compose.waitForIdle()
                }
            }
        }
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onNodeWithContentDescriptionCompat(
        contentDescription: String,
    ) = onNode(androidx.compose.ui.test.hasContentDescription(contentDescription))
}
