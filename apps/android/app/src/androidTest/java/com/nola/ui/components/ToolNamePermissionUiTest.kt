// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.components

import android.content.res.Configuration
import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.ui.theme.NolaTheme
import java.io.File
import java.util.Locale
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ToolNamePermissionUiTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun localizedToolNameInPermissionRequestCanBeDenied() {
        val activity = compose.activity
        val configuration = Configuration(activity.resources.configuration).apply { setLocale(Locale.SIMPLIFIED_CHINESE) }
        val context = activity.createConfigurationContext(configuration)
        var denied = false
        compose.setContent {
            CompositionLocalProvider(LocalConfiguration provides configuration, LocalContext provides context) {
                NolaTheme {
                    PermissionDialog(
                        toolName = "delete_file", message = "notes.txt", executionLocation = context.getString(R.string.permission_location_device),
                        dataBoundary = context.getString(R.string.permission_data_device),
                        validity = context.getString(R.string.permission_valid_once),
                        allowLabel = context.getString(R.string.permission_action_allow_once),
                        onAllow = { error("Unexpected permission grant") }, onDeny = { denied = true },
                    )
                }
            }
        }
        compose.onNodeWithText("删除文件", substring = true).assertIsDisplayed()
        if (InstrumentationRegistry.getArguments().getString("screenshots") == "true") {
            val bitmap = compose.onRoot().captureToImage().asAndroidBitmap()
            File(activity.cacheDir, "tool-name-permission.png").outputStream().use {
                assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
            }
        }
        compose.onNodeWithText(context.getString(R.string.common_deny)).performClick()
        compose.runOnIdle { assertTrue(denied) }
    }
}
