// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.onboarding

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
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
        try {
            for (setup in listOf(true, false)) {
                runBlocking {
                    store.edit { it.remove(completed); it[plannedSources] = "calendar,documents" }
                }
                ActivityScenario.launch(MainActivity::class.java).use { activity ->
                    val action = context.getString(if (setup) R.string.onboarding_choose_model else R.string.onboarding_guide_action_later)
                    compose.waitUntil(15_000) { compose.onAllNodesWithText(action).fetchSemanticsNodes().isNotEmpty() }
                    compose.onNodeWithText(action).performClick()
                    if (setup) {
                        waitForScreen("models.refresh")
                        activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
                    }
                    waitForScreen("chat.conversations.list")
                }
                ActivityScenario.launch(MainActivity::class.java).use {
                    waitForScreen("chat.conversations.list")
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
