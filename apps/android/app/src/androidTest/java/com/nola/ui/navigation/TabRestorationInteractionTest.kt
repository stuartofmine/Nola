// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.navigation

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
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
import kotlinx.coroutines.flow.first
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
        val preferences = PreferenceDataStore(context, SecureTokenStore(context))
        val (url, token, pin, hostId) = runBlocking {
            preferences.setOnboardingCompleted()
            listOf(preferences.webHostBaseUrl.first(), preferences.webHostToken.first(),
                preferences.webHostCertificateSha256.first(), preferences.webHostIdentity.first())
        }
        try {
            runBlocking { preferences.clearWebHostConfig() }
            verifyModelNavigation(context)
        } finally {
            if (url.isNotBlank() && token.isNotBlank()) runBlocking {
                preferences.saveWebHostConfig(url, token, pin, hostId.ifBlank { null })
            }
        }
    }

    private fun verifyModelNavigation(context: android.content.Context) {
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
            val draft = "audit-draft"
            compose.onNodeWithTag("model.setup.modelID").performScrollTo().performTextReplacement(draft)
            fun sourceTab(label: Int) = compose.onNode(hasText(context.getString(label)) and hasClickAction()
                and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab))
            compose.onNodeWithText(context.getString(R.string.model_connections_account_details)).performScrollTo().performClick()
            compose.onNodeWithText(context.getString(R.string.model_connections_source_boundary)).assertExists()
            compose.onNodeWithText(context.getString(R.string.onboarding_guide_runtime_host)).performScrollTo().performClick()
            compose.onNodeWithText(context.getString(R.string.models_web_host_paste_setup)).assertExists()
            InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            compose.waitForIdle()
            sourceTab(R.string.models_web_host).assertIsSelected()
            tab(R.string.tab_activity).performClick()
            compose.onNodeWithTag("models.refresh").assertDoesNotExist()
            tab(R.string.tab_settings).performClick()
            compose.onNodeWithTag("models.refresh").assertIsDisplayed()
            sourceTab(R.string.models_web_host).assertIsSelected()
            sourceTab(R.string.models_service_tab).performScrollTo().performClick().assertIsSelected()
            compose.onNodeWithTag("model.setup.modelID").performScrollTo().assertTextContains(draft)
            tab(R.string.tab_settings).performClick()
            compose.onNodeWithTag("models.refresh").assertDoesNotExist()
            compose.onNodeWithText(context.getString(R.string.models_title)).assertExists()
        }
    }
}
