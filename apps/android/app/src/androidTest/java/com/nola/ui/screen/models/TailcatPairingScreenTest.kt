// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.models

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.net.URI

class TailcatPairingScreenTest : TailcatPairingScreenFixture() {

    @Test fun pairInTheAppThenVerifyAgainThroughTailcat() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("liveTailcatUi") == "1")
        verifyPairedHost(expectTailcat = true)
    }

}

class TailcatUnavailablePairingScreenTest : TailcatPairingScreenFixture() {
    @Test fun unavailableTailcatExplainsRejectedDirectConnectionAndKeepsPairing() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("liveTailcatUnavailableUi") == "1")
        verifyPairedHost(expectTailcat = false)
    }

}

abstract class TailcatPairingScreenFixture {
    @get:Rule val compose = createEmptyComposeRule()

    protected fun verifyPairedHost(expectTailcat: Boolean) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        requireIsolatedProductTestUser(context)
        val fixture = JSONObject(File(context.filesDir, "tailcat-live.json").readText())
        val origin = fixture.getString("origin")
        require(URI(origin).host in setOf("10.0.2.2", "127.0.0.1"))
        val prefs = PreferenceDataStore(context, SecureTokenStore(context))
        runBlocking { check(prefs.webHostBaseUrl.first().isBlank()); prefs.setOnboardingCompleted() }
        fun text(id: Int) = compose.onNodeWithText(context.getString(id))
        fun sheetShown() = compose.onAllNodesWithTag("models.host.heading").fetchSemanticsNodes().isNotEmpty()
        fun capture(name: String) {
            compose.waitForIdle()
            val automation = instrumentation.uiAutomation
            automation.waitForIdle(500, 6_000)
            // A departing sheet and the platform dialog can keep animating after the new semantics tree is committed.
            var bitmap = automation.takeScreenshot()
            var steadyFrames = 0
            val deadline = android.os.SystemClock.elapsedRealtime() + 5_000
            while (steadyFrames < 3 && android.os.SystemClock.elapsedRealtime() < deadline) {
                android.os.SystemClock.sleep(180)
                val next = automation.takeScreenshot()
                steadyFrames = if (bitmap.sameAs(next)) steadyFrames + 1 else 0
                bitmap.recycle()
                bitmap = next
            }
            check(steadyFrames == 3) { "The screenshot did not settle" }
            try {
                File(context.filesDir, "tailcat-$name.png").outputStream().use {
                    assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                }
            } finally { bitmap.recycle() }
        }
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(15_000) { compose.onAllNodesWithTag("navigation.bottom").fetchSemanticsNodes().isNotEmpty() }
                compose.onNode(hasContentDescription(context.getString(R.string.tab_settings)) and
                    hasAnyAncestor(hasTestTag("navigation.bottom"))).performClick()
                text(R.string.models_title).performScrollTo().performClick()
                compose.onNode(hasText(context.getString(R.string.models_web_host)) and isSelectable()).performClick()
                val connection = compose.onNodeWithTag(MODEL_HOST_CONNECTION_TEST_TAG)
                connection.performScrollTo().performClick()
                text(R.string.models_web_host_setup_title).assertIsDisplayed()
                capture("connection-form")
                compose.onNodeWithTag("models.host.url").performScrollTo().performClick().performTextReplacement(origin)
                val token = compose.onNodeWithTag("models.host.token")
                token.performScrollTo().performClick().performTextReplacement(fixture.getString("token"))
                token.performImeAction()
                compose.waitUntil(50_000) { !sheetShown() }
                assertEquals(fixture.getString("host"), runBlocking { prefs.webHostIdentity.first() })
                connection.performScrollTo().performClick()
                text(R.string.models_verify_again).performClick()
                if (!expectTailcat) {
                    val guidance = context.getString(R.string.models_direct_verify_failed_without_tailcat, 503)
                    compose.waitUntil(50_000) { compose.onAllNodesWithText(guidance).fetchSemanticsNodes().isNotEmpty() }
                    compose.onAllNodesWithText(guidance).onFirst().assertIsDisplayed()
                    text(R.string.models_verify_again).assertIsEnabled()
                    assertTrue(sheetShown())
                    assertEquals(fixture.getString("host"), runBlocking { prefs.webHostIdentity.first() })
                    capture("unavailable-direct")
                    return@use
                }
                compose.waitUntil(50_000) { !sheetShown() }
                text(R.string.models_web_host_verified).assertIsDisplayed()
                compose.onNodeWithTag("models.host.verified-transport").performScrollTo()
                    .assertTextEquals(context.getString(R.string.models_verified_via_tailcat))
                capture("verified-host")
                val disconnect = compose.onNodeWithTag("models.web_host.disconnect")
                disconnect.performScrollTo().performClick()
                text(R.string.chat_host_pairing_forget_confirm_title).assertIsDisplayed()
                capture("disconnect-confirmation")
                text(R.string.chat_host_pairing_forget_confirm_action).performClick()
                compose.waitUntil(35_000) { File(context.filesDir, "tailcat-lease-closed").isFile }
                assertTrue(runBlocking { prefs.webHostBaseUrl.first().isBlank() })
                assertTrue(runBlocking { prefs.webHostToken.first().isBlank() })
            }
        } finally {
            if (runBlocking { prefs.webHostBaseUrl.first() } == origin) runBlocking { prefs.clearWebHostConfig() }
        }
    }
}
