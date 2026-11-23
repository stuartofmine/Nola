// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.models

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.view.accessibility.AccessibilityNodeInfo
import android.content.Context
import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.domain.model.ModelInfo
import com.nola.ui.screen.chat.AndroidExpertNativeTestDependencies
import dagger.hilt.android.EntryPointAccessors
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

class HostKeyboardProductTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val context: Context get() = instrumentation.targetContext
    private val prefs get() = PreferenceDataStore(context, SecureTokenStore(context))
    private fun label(resource: Int) = context.getString(resource)
    private fun tag(id: String) = compose.onNodeWithTag(id)
    private fun tab(id: Int) = compose.onNode(hasContentDescription(label(id)) and hasAnyAncestor(hasTestTag("navigation.bottom")))
    private fun shown(id: String) = compose.onAllNodesWithTag(id).fetchSemanticsNodes().isNotEmpty()
    private fun awaitIme(visible: Boolean) {
        compose.waitUntil(8_000) {
            val fd = instrumentation.uiAutomation.executeShellCommand("dumpsys input_method")
            val dump = android.os.ParcelFileDescriptor.AutoCloseInputStream(fd).bufferedReader().use { it.readText() }
            ("mInputShown=true" in dump) == visible
        }
    }

    private fun capture(name: String) {
        instrumentation.uiAutomation.waitForIdle(250, 5_000)
        val output = File(context.filesDir, "keyboard-host-ux/$name.png")
        output.parentFile!!.mkdirs()
        val bitmap = instrumentation.uiAutomation.takeScreenshot()
        try { output.outputStream().use { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) } }
        finally { bitmap.recycle() }
    }

    /** Verify content inside the external browser before returning to the owning Nola screen. */
    private fun verifyBrowserCatalog(host: String, target: String, marker: String, screenshot: String) {
        val automation = instrumentation.uiAutomation
        automation.serviceInfo = automation.serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
        }
        fun browserNode(id: String) = automation.rootInActiveWindow
            ?.findAccessibilityNodeInfosByViewId("com.android.chrome:id/$id")?.firstOrNull()
        fun shell(command: String) {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command)).use { it.readBytes() }
        }
        require(marker.isNotBlank())
        val expected = android.net.Uri.parse(host + target)
        var lastShortcut = 0L
        compose.waitUntil(60_000) {
            // A fresh Chrome activity initially covers the destination with its own first-run UI.
            // Only dismiss known Chrome prompts on the isolated test device.
            val dismiss = browserNode("signin_fre_dismiss_button")
                ?: browserNode("negative_button")?.takeIf { browserNode("modal_dialog_view") != null && browserNode("positive_button") != null }
                ?: browserNode("button_secondary")?.takeIf { browserNode("promo_dialog_layout") != null }
            when {
                dismiss != null -> { dismiss.performAction(AccessibilityNodeInfo.ACTION_CLICK); false }
                browserNode("find_query") != null -> true
                else -> {
                    val address = browserNode("url_bar")?.text?.toString().orEmpty()
                    val actual = android.net.Uri.parse(if ("://" in address) address else "http://$address")
                    val now = android.os.SystemClock.elapsedRealtime()
                    if (actual.encodedAuthority == expected.encodedAuthority && actual.encodedPath == expected.encodedPath &&
                        actual.encodedQuery == expected.encodedQuery && now - lastShortcut >= 1500) {
                        shell("input keycombination KEYCODE_CTRL_LEFT KEYCODE_F")
                        lastShortcut = now
                    }
                    false
                }
            }
        }
        val query = android.os.Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, marker)
        }
        val emptyQuery = android.os.Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, "")
        }
        var lastSearch = 0L
        compose.waitUntil(45_000) {
            // A search started before an asynchronously rendered catalog can keep its initial
            // zero results even after the catalog appears. Reissue the full query in that case.
            val field = browserNode("find_query")
            val count = browserNode("find_status")?.text?.toString()
            val now = android.os.SystemClock.elapsedRealtime()
            when {
                field == null -> false
                field.text?.toString() != marker -> {
                    field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, query)
                    lastSearch = now
                    false
                }
                count?.matches(Regex("[1-9][0-9]*/[1-9][0-9]*")) == true -> true
                now - lastSearch > 2_000 -> {
                    field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, emptyQuery)
                    lastSearch = now
                    false
                }
                else -> false
            }
        }
        assertTrue(browserNode("close_find_button")?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true)
        compose.waitUntil(5_000) { browserNode("find_query") == null }
        capture(screenshot)
        assertTrue(automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK))
        compose.waitUntil(10_000) { automation.rootInActiveWindow?.packageName == context.packageName }
    }

    @Test fun blankSearchTapHidesSystemImeAndKeepsTheQuery() {
        requireIsolatedProductTestUser(context)
        runBlocking { prefs.setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(10_000) { shown("chat.conversations.search") }
            val field = tag("chat.conversations.search")
            field.performTouchInput { click() }
            awaitIme(true)
            field.performTextReplacement("unique draft")
            field.performTouchInput { click(centerLeft + androidx.compose.ui.geometry.Offset(30f, 0f)) }
            awaitIme(true)
            compose.onNodeWithText(label(R.string.conversations_title)).performTouchInput { click() }
            awaitIme(false)
            field.assertTextContains("unique draft")
            compose.onNodeWithText(label(R.string.conversations_no_results)).assertIsDisplayed()
            capture("search-preserved")
        }
    }

    @Test fun conversationBlankAreaDismissesKeyboardAndPreservesTheUnsentMessage() {
        requireIsolatedProductTestUser(context)
        runBlocking { prefs.setOnboardingCompleted() }
        ActivityScenario.launch(MainActivity::class.java).use {
            compose.waitUntil(10_000) { shown("chat.conversations.search") }
            compose.onNodeWithContentDescription(label(R.string.chat_new_chat)).performTouchInput { click() }
            val field = tag("chat.composer.input").assertIsEnabled()
            // Tap the visible leading padding as a user can, not just the text glyphs.
            field.performTouchInput { click(centerLeft + androidx.compose.ui.geometry.Offset(5f, 0f)) }
            field.assertIsFocused()
            awaitIme(true)
            field.performTextReplacement("draft survives keyboard dismissal")
            tag("chat-transcript-list").performTouchInput { click(androidx.compose.ui.geometry.Offset(4f, 4f)) }
            awaitIme(false)
            field.assertTextContains("draft survives keyboard dismissal")
            capture("chat-unsent-draft")
        }
    }

    @Test fun conversationSettingsObservesPairingAddedAfterLaunchAndCanForgetIt() {
        requireIsolatedProductTestUser(context)
        val url = "http://127.0.0.1:1"
        val hostModel = ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost)
        runBlocking { check(prefs.webHostBaseUrl.first().isBlank()); prefs.setOnboardingCompleted() }
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(10_000) { shown("chat.conversations.search") }
                runBlocking {
                    prefs.saveWebHostConfig(url, "isolated-settings-token", verifiedHostId = "isolated-settings-host")
                    prefs.saveDefaultModelSelection(hostModel)
                    prefs.setHybridDeviceToolsEnabled(true)
                }
                compose.onNodeWithContentDescription(label(R.string.chat_new_chat)).performClick()
                tag("chat.topbar.more").performTouchInput { click() }
                compose.onNodeWithText(label(R.string.inspector_settings)).performScrollTo().performClick()
                val forget = compose.onNodeWithText(label(R.string.chat_host_pairing_forget))
                forget.performScrollTo().assertIsDisplayed().performClick()
                compose.onNodeWithText(label(R.string.common_cancel)).performClick()
                assertEquals("isolated-settings-host", runBlocking { prefs.webHostIdentity.first() })
                capture("conversation-settings-pairing")
                forget.performClick()
                compose.onNodeWithText(label(R.string.chat_host_pairing_forget_confirm_action)).performClick()
                compose.waitUntil(10_000) { runBlocking { prefs.webHostBaseUrl.first().isBlank() } }
                forget.assertDoesNotExist()
                assertNull(runBlocking { prefs.defaultModelSelection(listOf(hostModel)) })
                assertTrue(runBlocking { prefs.webHostToken.first().isEmpty() })
                assertFalse(runBlocking { prefs.hybridDeviceToolsEnabled.first() })
                capture("conversation-forgotten")
            }
        } finally {
            if (runBlocking { prefs.webHostBaseUrl.first() } == url) runBlocking { prefs.clearWebHostConfig() }
        }
    }

    @Test fun replacingActivePairingRevokesTheOldRuntimeBeforeExplicitlySelectingTheNewHost() {
        requireIsolatedProductTestUser(context)
        val files = listOf("mobile-ux-host.json", "mobile-ux-host-other.json").map { File(context.filesDir, it) }
        assumeTrue("Stage two independent loopback Nola Hosts on the disposable device", files.all { it.isFile })
        val hosts = files.map { JSONObject(it.readText()) }
        require(hosts.all { java.net.URI(it.getString("url")).host == "127.0.0.1" })
        val dependencies = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java)
        val service = dependencies.modelService()
        val provider = dependencies.webAgentProvider()
        val hostModel = ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost)
        fun hostDefault() = runBlocking { prefs.defaultModelSelection(listOf(hostModel)) }
        runBlocking { check(prefs.webHostBaseUrl.first().isBlank()); prefs.setOnboardingCompleted() }
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(10_000) { shown("navigation.bottom") }
                tab(R.string.tab_settings).performTouchInput { click() }
                compose.onNodeWithText(label(R.string.models_title)).performScrollTo().performClick()
                compose.onNode(hasText(label(R.string.models_web_host)) and isSelectable()).performClick()
                val connection = tag(MODEL_HOST_CONNECTION_TEST_TAG)
                val token = tag("models.host.token")
                fun pair(host: JSONObject) {
                    connection.performScrollTo().performClick()
                    tag("models.host.url").performScrollTo().performClick().performTextReplacement(host.getString("url"))
                    compose.onNodeWithText(label(R.string.models_save_and_verify)).assertIsNotEnabled()
                    token.performScrollTo().performClick().performTextReplacement(host.getString("token"))
                    token.performImeAction()
                    compose.waitUntil(15_000) { !shown("models.host.heading") }
                }
                fun activate() {
                    compose.onNodeWithText(label(R.string.models_use)).performScrollTo().performClick()
                    compose.waitUntil(10_000) { hostDefault() != null && provider.config != null }
                    assertEquals("web-host", service.currentModelSource)
                    compose.waitUntil(10_000) {
                        compose.onAllNodesWithText(label(R.string.models_selected)).fetchSemanticsNodes().isNotEmpty() &&
                            compose.onAllNodesWithText(label(R.string.models_use)).fetchSemanticsNodes().isEmpty()
                    }
                    compose.onNodeWithText(label(R.string.models_selected)).assertIsDisplayed()
                }
                pair(hosts[0])
                assertNull(hostDefault())
                activate()
                val firstIdentity = runBlocking { prefs.webHostIdentity.first() }
                capture("selected-host")
                connection.performScrollTo().performClick()
                compose.onNodeWithText(label(R.string.models_verify_again)).performClick()
                compose.waitUntil(15_000) { !shown("models.host.heading") }
                compose.onNodeWithText(label(R.string.models_web_host_default_verified)).assertIsDisplayed()
                assertEquals(firstIdentity, provider.config?.expectedHostId)
                assertNotNull(hostDefault())
                val arrived = java.util.concurrent.CountDownLatch(1)
                val release = java.util.concurrent.CountDownLatch(1)
                java.net.ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { pending ->
                    val worker = kotlin.concurrent.thread(isDaemon = true) {
                        runCatching {
                            pending.accept().use { socket ->
                                socket.getInputStream().bufferedReader().readLine()
                                arrived.countDown()
                                release.await(10, java.util.concurrent.TimeUnit.SECONDS)
                            }
                        }
                    }
                    try {
                        connection.performScrollTo().performClick()
                        tag("models.host.url").performScrollTo().performClick()
                            .performTextReplacement("http://127.0.0.1:${pending.localPort}")
                        token.performScrollTo().performClick().performTextReplacement("disposable-cancelled-request")
                        token.performImeAction()
                        compose.waitUntil(5_000) { arrived.count == 0L }
                        compose.onNodeWithText(label(R.string.models_cancel_host_verification)).performClick()
                        release.countDown()
                        worker.join(2_000)
                        compose.onNodeWithText(label(R.string.models_save_and_verify)).assertIsEnabled()
                        tag("models.host.feedback").assertDoesNotExist()
                        compose.onNodeWithText(label(R.string.common_cancel)).performClick()
                        compose.onNodeWithText(label(R.string.models_discard_changes_action)).performClick()
                        assertEquals(firstIdentity, runBlocking { prefs.webHostIdentity.first() })
                        assertTrue(provider.config?.token == hosts[0].getString("token"))
                        assertNotNull(hostDefault())
                    } finally {
                        release.countDown()
                    }
                }
                runBlocking {
                    prefs.setHybridDeviceToolsEnabled(true)
                    prefs.setHybridDeviceCapability("fixture-camera", true)
                }
                pair(hosts[1])
                assertNotEquals(firstIdentity, runBlocking { prefs.webHostIdentity.first() })
                assertNull(hostDefault())
                assertNull(provider.config)
                assertTrue(service.isPlaceholder)
                assertFalse(runBlocking { prefs.hybridDeviceToolsEnabled.first() })
                assertTrue(runBlocking { prefs.hybridDeviceCapabilities.first().isEmpty() })
                capture("replacement-awaits-selection")
                activate()
                assertEquals(hosts[1].getString("url"), provider.config?.hostBaseUrl)
                val disconnect = tag("models.web_host.disconnect")
                disconnect.performScrollTo().performClick()
                compose.onNodeWithText(label(R.string.chat_host_pairing_forget_confirm_message)).assertIsDisplayed()
                capture("disconnect-confirmation")
                compose.onNodeWithText(label(R.string.common_cancel)).performClick()
                assertNotNull(hostDefault())
                assertEquals(hosts[1].getString("url"), provider.config?.hostBaseUrl)
                disconnect.performScrollTo().performClick()
                compose.onNodeWithText(label(R.string.chat_host_pairing_forget_confirm_action)).performClick()
                compose.waitUntil(10_000) { !shown("models.web_host.disconnect") }
                assertNull(hostDefault())
                assertNull(provider.config)
                assertTrue(service.isPlaceholder)
                assertTrue(runBlocking { prefs.webHostToken.first().isBlank() })
            }
        } finally {
            runBlocking { if (hosts.any { it.getString("url") == prefs.webHostBaseUrl.first() }) prefs.clearWebHostConfig() }
            provider.config = null
            if (service.currentModelSource == "web-host") service.usePlaceholder()
        }
    }

    @Test fun rejectedThenVerifiedHostKeepsDraftAndShowsSavedResultInHostTab() {
        requireIsolatedProductTestUser(context)
        val fixture = File(context.filesDir, "mobile-ux-host.json")
        assumeTrue("Start the isolated loopback Nola Host and stage the fixture", fixture.isFile)
        val owned = JSONObject(fixture.readText())
        val url = owned.getString("url")
        require(java.net.URI(url).host == "127.0.0.1")
        val accessToken = owned.getString("token")
        runBlocking { check(prefs.webHostBaseUrl.first().isBlank()); prefs.setOnboardingCompleted() }
        try {
            ActivityScenario.launch(MainActivity::class.java).use {
                compose.waitUntil(10_000) { shown("navigation.bottom") }
                tab(R.string.tab_settings).performTouchInput { click() }
                compose.onNodeWithText(label(R.string.models_title)).performScrollTo().performTouchInput { click() }
                compose.onNode(hasText(label(R.string.models_web_host)) and isSelectable()).performTouchInput { click() }
                tag(MODEL_HOST_CONNECTION_TEST_TAG).performScrollTo().performTouchInput { click() }
                val address = tag("models.host.url")
                address.performScrollTo().performTouchInput { click() }
                awaitIme(true)
                address.performTextReplacement(url)
                tag("models.host.certificateSha256").assertDoesNotExist()
                address.performTouchInput { longClick() }
                awaitIme(true)
                address.performTouchInput { click() }
                val scroller = tag("models.host.fields")
                scroller.performTouchInput { swipeUp() }
                assertTrue(scroller.fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.VerticalScrollAxisRange].value() > 0f)
                awaitIme(true)
                tag("models.host.heading").performTouchInput { click() }
                awaitIme(false)
                address.assertTextContains(url)
                capture("host-draft")
                val token = tag("models.host.token")
                address.performScrollTo().performTouchInput { click() }
                address.performImeAction()
                token.assertIsFocused()
                awaitIme(true)
                token.performTextReplacement("intentionally-rejected-fixture")
                val save = compose.onNodeWithText(label(R.string.models_save_and_verify))
                compose.onNodeWithText(label(R.string.models_host_token_new)).assertDoesNotExist()
                token.performImeAction()
                awaitIme(false)
                compose.waitUntil(15_000) { shown("models.host.feedback") }
                tag("models.host.feedback").assertTextContains(label(R.string.models_error_host_unauthorized)).assertIsDisplayed()
                save.assertIsEnabled()
                address.assertTextContains(url)
                assertTrue(runBlocking { prefs.webHostBaseUrl.first().isBlank() })
                capture("host-rejected-retry")
                token.performScrollTo().performTouchInput { click() }
                token.performTextReplacement(accessToken)
                tag("models.host.feedback").assertDoesNotExist()
                save.performTouchInput { click() }
                compose.waitUntil(15_000) { !shown("models.host.heading") && shown("models.web_host.disconnect") }
                val result = compose.onNodeWithText(label(R.string.models_web_host_verified))
                result.assertIsDisplayed()
                assertEquals(url, runBlocking { prefs.webHostBaseUrl.first() })
                assertTrue(runBlocking { prefs.webHostIdentity.first().isNotBlank() })
                capture("host-verified")
                tag("models.web_host.manage").performScrollTo().performTouchInput { click() }
                verifyBrowserCatalog(url, "/models?tab=api", owned.getString("managementModel"), "paired-host-live-catalog")
                compose.onNode(hasText(label(R.string.models_web_host)) and isSelectable()).performScrollTo().performTouchInput { click() }
                tag("models.web_host.disconnect").performScrollTo().performTouchInput { click() }
                compose.onNodeWithText(label(R.string.chat_host_pairing_forget_confirm_action)).performTouchInput { click() }
                compose.waitUntil(10_000) { !shown("models.web_host.disconnect") }
                assertTrue(runBlocking { prefs.webHostBaseUrl.first().isBlank() })
                capture("host-disconnected")
            }
        } finally {
            if (runBlocking { prefs.webHostBaseUrl.first() } == url) runBlocking { prefs.clearWebHostConfig() }
        }
    }

    @Test fun pairedHostPluginDirectoryReturnsToSelectedPluginTab() {
        requireIsolatedProductTestUser(context)
        val fixture = File(context.filesDir, "mobile-ux-host.json")
        assumeTrue("Start the isolated Nola Host", fixture.isFile)
        val owned = JSONObject(fixture.readText())
        val marker = owned.optString("managementPluginText")
        assumeTrue("Stage a visible plugin catalog marker for this Host", marker.isNotBlank())
        val url = owned.getString("url")
        require(java.net.URI(url).host == "127.0.0.1")
        runBlocking { check(prefs.webHostBaseUrl.first().isBlank()); prefs.setOnboardingCompleted() }
        try {
            ActivityScenario.launch(MainActivity::class.java).use { activity ->
                compose.waitUntil(10_000) { shown("navigation.bottom") }
                tab(R.string.tab_settings).performTouchInput { click() }
                compose.onNodeWithText(label(R.string.models_title)).performScrollTo().performClick()
                compose.onNode(hasText(label(R.string.models_web_host)) and isSelectable()).performClick()
                tag(MODEL_HOST_CONNECTION_TEST_TAG).performScrollTo().performClick()
                val address = tag("models.host.url")
                address.performScrollTo().performTouchInput { click() }
                address.performTextReplacement(url)
                address.performImeAction()
                val token = tag("models.host.token")
                token.assertIsFocused().performTextReplacement(owned.getString("token"))
                val save = compose.onNodeWithText(label(R.string.models_save_and_verify))
                save.assertIsEnabled().performTouchInput { click() }
                compose.waitUntil(30_000) { shown("models.web_host.disconnect") || shown("models.host.feedback") }
                compose.onNodeWithText(label(R.string.models_web_host_verified)).assertIsDisplayed()
                check(runBlocking { prefs.webHostBaseUrl.first() == url })
                activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
                compose.onNodeWithText(label(R.string.settings_capabilities)).performScrollTo().performClick()
                val plugins = tag("intelligence.capabilities.plugins")
                plugins.performClick().assertIsSelected()
                tag("intelligence.harness.plugins").performScrollTo().performTouchInput { click() }
                verifyBrowserCatalog(url, "/capabilities?tab=plugins", marker, "paired-host-plugin-catalog")
                plugins.assertIsSelected()
                tag("intelligence.harness.plugins").assertIsDisplayed()
                capture("returned-to-plugins")
            }
        } finally {
            if (runBlocking { prefs.webHostBaseUrl.first() } == url) runBlocking { prefs.clearWebHostConfig() }
        }
    }
}
