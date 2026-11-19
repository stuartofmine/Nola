// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.theme

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.ui.WarmPaletteExpectations
import com.nola.ui.assertDominantWarmSurface
import com.nola.ui.assertRenderedBackgroundMatches
import com.nola.ui.assertRenderedTextColor
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaPage
import com.nola.ui.screen.chat.PersonalIntelligenceDialog
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class WarmThemePixelAndContrastUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun nolaPageNolaCardZeroElevationAndStandaloneTextRenderWarmPaperAndInk() {
        var darkTheme by mutableStateOf(false)
        compose.setContent {
            NolaTheme(darkTheme = darkTheme) {
                Column(Modifier.width(360.dp).height(640.dp)) {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(MaterialTheme.colorScheme.background)
                            .padding(16.dp)
                            .testTag("theme.standalone.box"),
                    ) {
                        Text(
                            text = "Standalone Warm Text",
                            modifier = Modifier.testTag("theme.standalone.text"),
                        )
                    }
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(100.dp)
                            .testTag("theme.flat.surface"),
                        color = MaterialTheme.colorScheme.surface,
                        tonalElevation = 0.dp,
                    ) {
                        Box(Modifier.padding(16.dp), contentAlignment = Alignment.CenterStart) {
                            Text("Zero tonal elevation surface")
                        }
                    }
                    NolaPage(modifier = Modifier.weight(1f).testTag("theme.page")) {
                        NolaCard(modifier = Modifier.testTag("theme.card"), plain = true) {
                            Text(
                                text = "Card body text",
                                modifier = Modifier.testTag("theme.card.text"),
                            )
                        }
                    }
                }
            }
        }

        for (isDark in listOf(false, true)) {
            compose.runOnIdle { darkTheme = isDark }
            compose.waitForIdle()

            val expectedBg = if (isDark) WarmPaletteExpectations.DarkBackground else WarmPaletteExpectations.LightBackground
            val expectedSurface = if (isDark) WarmPaletteExpectations.DarkSurface else WarmPaletteExpectations.LightSurface
            val expectedText = if (isDark) WarmPaletteExpectations.DarkText else WarmPaletteExpectations.LightText

            compose.onNodeWithTag("theme.page")
                .assertRenderedBackgroundMatches(expectedBg, isDark, "NolaPage(${if (isDark) "dark" else "light"})")
            compose.onNodeWithTag("theme.flat.surface")
                .assertRenderedBackgroundMatches(expectedSurface, isDark, "FlatSurface(${if (isDark) "dark" else "light"})")
            compose.onNodeWithTag("theme.card")
                .assertRenderedBackgroundMatches(expectedSurface, isDark, "NolaCard(${if (isDark) "dark" else "light"})")
            compose.onNodeWithTag("theme.standalone.box")
                .assertRenderedBackgroundMatches(expectedBg, isDark, "StandaloneBox(${if (isDark) "dark" else "light"})")

            compose.onNodeWithTag("theme.standalone.text")
                .assertRenderedTextColor(expectedText, "StandaloneText(${if (isDark) "dark" else "light"})")
            compose.onNodeWithTag("theme.card.text")
                .assertRenderedTextColor(expectedText, "CardText(${if (isDark) "dark" else "light"})")
        }
    }

    @Test
    fun personalIntelligenceDialogRendersWarmPaperAndInkWithoutTonalTintOrBlackText() {
        var darkTheme by mutableStateOf(false)
        compose.setContent {
            NolaTheme(darkTheme = darkTheme) {
                Box(Modifier.size(390.dp, 844.dp)) {
                    PersonalIntelligenceDialog(onDismiss = {})
                }
            }
        }

        val title = compose.activity.getString(R.string.personal_intelligence_title)
        for (isDark in listOf(false, true)) {
            compose.runOnIdle { darkTheme = isDark }
            compose.waitForIdle()

            val expectedSurface = if (isDark) WarmPaletteExpectations.DarkSurface else WarmPaletteExpectations.LightSurface
            val expectedText = if (isDark) WarmPaletteExpectations.DarkText else WarmPaletteExpectations.LightText

            compose.onNode(isDialog())
                .captureToImage()
                .toPixelMap()
                .assertDominantWarmSurface(
                    darkTheme = isDark,
                    label = "PersonalIntelligenceDialog(${if (isDark) "dark" else "light"})",
                )
            compose.onNodeWithText(title, useUnmergedTree = true)
                .assertRenderedTextColor(expectedText, "PersonalIntelligenceTitle(${if (isDark) "dark" else "light"})")
        }
    }
}
