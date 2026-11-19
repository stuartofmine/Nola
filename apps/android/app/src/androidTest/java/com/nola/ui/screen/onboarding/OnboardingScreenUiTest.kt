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
import androidx.compose.ui.test.assertIsNotEnabled
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
    fun welcomeChoicesAndSaveFailureStayReachableAcrossFormFactorsAndThemes() {
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
        val chooseModel = compose.activity.getString(R.string.onboarding_choose_model)
        val title = compose.activity.getString(R.string.onboarding_welcome_title)
        val errorLabel = compose.activity.getString(R.string.onboarding_guide_save_error)

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

                compose.onNodeWithText(title).assertIsDisplayed()
                if (isDark) {
                    compose.onNodeWithText(title, useUnmergedTree = true)
                        .assertRenderedTextColor(WarmPaletteExpectations.DarkText, "WelcomeTitle")
                    compose.onNodeWithText(laterLabel, useUnmergedTree = true)
                        .assertNonBlackDarkText("WelcomeSkip")
                }
                for (label in listOf(laterLabel, errorLabel, chooseModel)) {
                    compose.onNodeWithText(label).assertIsDisplayed().assertFullyInside(rootBounds, label)
                }
            }
        }
    }

    @Test
    fun eachButtonSelectsItsDestinationAndSavingPreventsDuplicateRequests() {
        val choices = mutableListOf<Boolean>()
        var completing by mutableStateOf(false)
        compose.setContent {
            NolaTheme {
                OnboardingContent(OnboardingUiState(isCompleting = completing)) { choices.add(it) }
            }
        }
        val later = compose.activity.getString(R.string.onboarding_guide_action_later)
        compose.onNodeWithText(compose.activity.getString(R.string.onboarding_choose_model)).performClick()
        compose.onNodeWithText(later).performClick()
        compose.runOnIdle {
            org.junit.Assert.assertEquals(listOf(true, false), choices)
            completing = true
        }
        compose.onNodeWithText(later).assertIsNotEnabled()
        compose.onNodeWithText(compose.activity.getString(R.string.onboarding_guide_action_saving)).assertIsNotEnabled()
    }
}
