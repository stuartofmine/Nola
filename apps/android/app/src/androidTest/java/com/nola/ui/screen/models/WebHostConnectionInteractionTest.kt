// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.models

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.ui.theme.NolaTheme
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class WebHostConnectionInteractionTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun pasteRequiresExplicitConnectionAndFailedCheckCanBeRetried() {
        requireIsolatedProductTestUser(compose.activity)
        val context = compose.activity
        var calls = 0
        var dismissed = false
        var finish: ((Boolean, String) -> Unit)? = null
        val setup = """{"type":"nola.server.connection","protocol_version":1,"origin":"http://192.168.1.8:5001","host_id":"host-studio-fixture","access_token":"fixture-token"}"""
        compose.setContent {
            NolaTheme { WebHostConfigSheet("", "", "", onSave = { url, token, pin, identity, completion ->
                assertEquals("http://192.168.1.8:5001", url)
                assertEquals("fixture-token", token)
                assertEquals("", pin)
                assertEquals("host-studio-fixture", identity)
                calls++
                finish = completion
            }, onDismiss = { dismissed = true }) }
        }
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        compose.runOnIdle { clipboard.setPrimaryClip(ClipData.newPlainText("fixture", "{}")) }
        val paste = context.getString(R.string.models_web_host_paste_setup)
        compose.onNodeWithText(paste).performScrollTo().performClick()
        compose.onNodeWithText(context.getString(R.string.models_web_host_invalid_setup)).assertExists()
        compose.runOnIdle { clipboard.setPrimaryClip(ClipData.newPlainText("fixture", setup)) }
        compose.onNodeWithText(paste).performScrollTo().performClick()
        compose.runOnIdle { assertEquals(0, calls) }
        val save = context.getString(R.string.models_save_and_verify)
        compose.onNodeWithText(save).performClick()
        compose.onNodeWithText(paste).assertIsNotEnabled()
        compose.runOnIdle { requireNotNull(finish)(false, "Fixture host offline") }
        compose.onNodeWithText("Fixture host offline").assertExists()
        compose.onNodeWithText(save).performClick()
        compose.runOnIdle {
            assertEquals(2, calls)
            requireNotNull(finish)(true, "Verified")
            assertTrue(dismissed)
        }
    }

    @Test fun cancelledCheckIgnoresLateSuccessAndKeepsDraft() {
        requireIsolatedProductTestUser(compose.activity)
        val context = compose.activity
        var dismissed = false
        var cancelled = 0
        var finish: ((Boolean, String) -> Unit)? = null
        compose.setContent {
            NolaTheme { WebHostConfigSheet("http://studio.local:5001", "fixture-token", "",
                onSave = { _, _, _, _, completion -> finish = completion },
                onDismiss = { dismissed = true }, onCancelConnection = { cancelled++ }) }
        }
        val verify = context.getString(R.string.models_verify_again)
        compose.onNodeWithText(verify).performClick()
        compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
        compose.runOnIdle { requireNotNull(finish)(true, "Late success"); assertEquals(1, cancelled); assertFalse(dismissed) }
        compose.onNodeWithText(verify).assertIsEnabled()
    }
}
