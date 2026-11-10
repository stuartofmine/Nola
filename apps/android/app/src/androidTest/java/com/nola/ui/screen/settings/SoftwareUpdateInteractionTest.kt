// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertHasClickAction
import com.nola.R
import com.nola.BuildConfig
import java.io.File
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.core.AgentCoreNative
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import com.nola.ui.theme.NolaTheme
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test

class SoftwareUpdateInteractionTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    @Test fun apkIdentityMustMatchTheSignedOfferAndCurrentApplication() {
        requireIsolatedProductTestUser(compose.activity)
        val own = File(compose.activity.applicationInfo.sourceDir)
        validateDownloadedAndroidUpdate(compose.activity, own, BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE.toLong())
        assertFalse(runCatching {
            validateDownloadedAndroidUpdate(compose.activity, own, BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE.toLong() + 1)
        }.isSuccess)
        val other = File(InstrumentationRegistry.getInstrumentation().context.applicationInfo.sourceDir)
        assertFalse(runCatching {
            validateDownloadedAndroidUpdate(compose.activity, other, BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE.toLong())
        }.isSuccess)
    }

    @Test fun updaterExposesCheckAndAbiCannotInstall() {
        requireIsolatedProductTestUser(compose.activity)
        compose.setContent { NolaTheme { SoftwareUpdateControls() } }
        compose.onNodeWithText(compose.activity.getString(R.string.software_update_check)).assertHasClickAction()
        compose.onNodeWithText(compose.activity.getString(R.string.software_update_not_checked)).assertExists()
        val result = AgentCoreNative.nativeSoftwareUpdate("""{"operation":"install","target":"android-aarch64"}""".toByteArray())!!
        val reply = JSONObject(String(result, Charsets.UTF_8))
        assertFalse(reply.getBoolean("ok"))
        assertEquals("request_invalid",reply.getJSONObject("error").getString("code"))
    }
}
