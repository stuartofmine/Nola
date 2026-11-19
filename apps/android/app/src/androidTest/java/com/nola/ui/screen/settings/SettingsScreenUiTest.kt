// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.data.local.datastore.AppearancePreference
import com.nola.ui.assertSingleLineNoOverflow
import com.nola.ui.theme.NolaTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SettingsScreenUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun scrollingRootResetsSubpageScrollAndBackRestoresRootScrollPosition() {
        var appearance by mutableStateOf(AppearancePreference.System)
        compose.setContent {
            NolaTheme(darkTheme = appearance == AppearancePreference.Dark) {
                val detailState = remember { mutableStateOf<String?>(null) }
                // Bounded height ensures the root Settings page genuinely scrolls.
                Box(Modifier.width(360.dp).height(440.dp).testTag("settings.viewport")) {
                    SettingsContent(
                        state = SettingsUiState(
                            isLoading = false,
                            clientUuid = "12345678-1234-1234-1234-1234567890ab",
                            localDataSizeBytes = 1024L * 1024L,
                        ),
                        appearance = appearance,
                        onAppearanceChange = { appearance = it },
                        onRefresh = {},
                        detailState = detailState,
                    )
                }
            }
        }

        val scheduledLabel = compose.activity.getString(R.string.scheduled_title)
        val privacyLabel = compose.activity.getString(R.string.settings_privacy)
        val generalLabel = compose.activity.getString(R.string.settings_general)
        val backLabel = compose.activity.getString(R.string.common_back)

        fun currentScrollValue(): Float = compose.onNode(hasScrollAction()).fetchSemanticsNode()
            .config[SemanticsProperties.VerticalScrollAxisRange].value()

        compose.waitForIdle()
        // Scroll root Settings to the bottom card.
        compose.onNodeWithText(scheduledLabel).performScrollTo().assertIsDisplayed()
        val rootScrolledOffset = currentScrollValue()
        assertTrue("Settings root must be scrolled past 0 before entering subpage (got $rootScrolledOffset)", rootScrolledOffset > 0f)

        // Tap Privacy near the bottom: subpage must start at scroll 0 with unclipped header.
        compose.onNodeWithText(privacyLabel).performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals("Privacy subpage must start at scroll offset 0", 0f, currentScrollValue(), 0.5f)
        val privacyHeaderBounds = compose.onAllNodes(androidx.compose.ui.test.hasText(privacyLabel))
            .fetchSemanticsNodes().first().boundsInRoot
        assertTrue("Privacy subpage header must not be clipped above viewport (top=${privacyHeaderBounds.top})", privacyHeaderBounds.top >= 0f)

        // Tap Back: root scroll position must be restored.
        compose.onNode(hasContentDescription(backLabel)).performClick()
        compose.waitForIdle()
        assertEquals(
            "Returning from Privacy must restore root scroll position",
            rootScrolledOffset,
            currentScrollValue(),
            2f,
        )

        // Tap General: verify scroll starts at 0, header is unclipped, and Appearance segmented control fits single-line.
        compose.onNodeWithText(generalLabel).performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals("General subpage must start at scroll offset 0", 0f, currentScrollValue(), 0.5f)
        val generalHeaderBounds = compose.onNodeWithText(generalLabel).fetchSemanticsNode().boundsInRoot
        assertTrue("General subpage header must not be clipped (top=${generalHeaderBounds.top})", generalHeaderBounds.top >= 0f)

        for (choiceRes in listOf(R.string.appearance_system, R.string.appearance_light, R.string.appearance_dark)) {
            val label = compose.activity.getString(choiceRes)
            compose.onNodeWithText(label, useUnmergedTree = true)
                .assertSingleLineNoOverflow("Appearance segment '$label'")
        }

        // Back restores root again.
        compose.onNode(hasContentDescription(backLabel)).performClick()
        compose.waitForIdle()
        compose.onNodeWithText(generalLabel).assertIsDisplayed()
    }
}
