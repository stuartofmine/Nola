// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.health.AndroidHealthSummary
import com.nola.ui.WarmPaletteExpectations
import com.nola.ui.assertRenderedBackgroundMatches
import com.nola.ui.assertSingleLineNoOverflow
import com.nola.ui.theme.NolaTheme
import java.time.Instant
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DataWorkspaceScreenUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun dataSourceCardsKeepActionButtonsAndBadgesSingleLineAcrossWidthsAndThemes() {
        var widthDp by mutableStateOf(320)
        var darkTheme by mutableStateOf(false)
        var currentState by mutableStateOf(needsPermissionState())

        compose.setContent {
            NolaTheme(darkTheme = darkTheme) {
                Box(
                    modifier = Modifier
                        .width(widthDp.dp)
                        .height(800.dp)
                        .testTag("data.workspace.viewport"),
                ) {
                    DataSourcesContent(
                        state = currentState,
                        onRefresh = {},
                        onRequestHealthPermission = {},
                    )
                }
            }
        }

        val allowAccessLabel = compose.activity.getString(R.string.data_sources_action_allow)
        val refreshLabel = compose.activity.getString(R.string.data_sources_action_refresh)
        val accessNeededBadge = compose.activity.getString(R.string.data_sources_status_access_needed)
        val readyBadge = compose.activity.getString(R.string.data_sources_status_ready)
        val errorBadge = compose.activity.getString(R.string.data_sources_status_error)

        val scenarios = listOf(
            Triple(needsPermissionState(), accessNeededBadge, allowAccessLabel),
            Triple(readyState(), readyBadge, refreshLabel),
            Triple(errorState(), errorBadge, refreshLabel),
        )

        for (width in listOf(320, 360, 390, 840)) {
            for (isDark in listOf(false, true)) {
                for ((uiState, expectedBadge, expectedAction) in scenarios) {
                    compose.runOnIdle {
                        widthDp = width
                        darkTheme = isDark
                        currentState = uiState
                    }
                    compose.waitForIdle()

                    compose.onNodeWithTag("data.health.card").performScrollTo().assertIsDisplayed()
                    val expectedSurface = if (isDark) WarmPaletteExpectations.DarkSurface else WarmPaletteExpectations.LightSurface
                    compose.onNodeWithTag("data.health.card")
                        .assertRenderedBackgroundMatches(expectedSurface, isDark, "DataHealthCard(${width}dp,dark=$isDark,$expectedBadge)")

                    compose.onNode(
                        hasText(expectedBadge) and hasAnyAncestor(hasTestTag("data.health.card")),
                        useUnmergedTree = true,
                    ).assertIsDisplayed()
                        .assertSingleLineNoOverflow("Badge '$expectedBadge' at ${width}dp")

                    compose.onNode(
                        hasText(expectedAction) and hasAnyAncestor(hasTestTag("data.health.card")),
                        useUnmergedTree = true,
                    ).assertIsDisplayed()
                        .assertSingleLineNoOverflow("Action '$expectedAction' at ${width}dp")
                }
            }
        }
    }

    private fun needsPermissionState() = DataSourcesUiState(
        health = AndroidHealthSummary(
            availability = AndroidHealthAvailability.Available,
            permissionState = AndroidHealthPermissionState.Missing,
            missingPermissionCount = 4,
        ),
    )

    private fun readyState() = DataSourcesUiState(
        health = AndroidHealthSummary(
            availability = AndroidHealthAvailability.Available,
            permissionState = AndroidHealthPermissionState.Granted,
            totalSteps = 6_200,
            updatedAt = Instant.now(),
        ),
    )

    private fun errorState() = DataSourcesUiState(
        health = AndroidHealthSummary(
            availability = AndroidHealthAvailability.Available,
            permissionState = AndroidHealthPermissionState.Granted,
            error = "Health Connect provider read error",
        ),
    )
}
