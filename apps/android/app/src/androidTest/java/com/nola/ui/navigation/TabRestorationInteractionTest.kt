// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.navigation

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.runBlocking
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Actual production NavHost and bottom tabs; no provider, credential or model request. */
@RunWith(AndroidJUnit4::class)
class TabRestorationInteractionTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun switchingTabsRestoresModelPageAndReselectingSettingsReturnsToItsRoot() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        runBlocking { PreferenceDataStore(context, SecureTokenStore(context)).setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(15_000) {
                compose.onAllNodesWithTag("navigation.bottom").fetchSemanticsNodes().isNotEmpty()
            }
            fun tab(label: Int) = compose.onNode(hasContentDescription(context.getString(label)) and
                hasAnyAncestor(hasTestTag("navigation.bottom")))
            tab(R.string.tab_settings).performClick()
            compose.onNodeWithText(context.getString(R.string.models_title)).performScrollTo().performClick()
            compose.waitUntil(15_000) { compose.onAllNodesWithTag("models.refresh").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag("models.refresh").assertIsDisplayed()
            tab(R.string.tab_activity).performClick()
            compose.onNodeWithTag("models.refresh").assertDoesNotExist()
            tab(R.string.tab_settings).performClick()
            compose.onNodeWithTag("models.refresh").assertIsDisplayed()
            tab(R.string.tab_settings).performClick()
            compose.onNodeWithTag("models.refresh").assertDoesNotExist()
            compose.onNodeWithText(context.getString(R.string.models_title)).assertExists()
        }
    }
}
