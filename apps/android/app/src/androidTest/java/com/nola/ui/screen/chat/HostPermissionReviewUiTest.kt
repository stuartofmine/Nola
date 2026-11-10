// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.nola.R
import com.nola.ui.components.PermissionDialog
import com.nola.ui.theme.NolaTheme
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class HostPermissionReviewUiTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    @Test fun missingDetailsDisableBothAllowChoicesButKeepDenyAvailable() {
        requireIsolatedProductTestUser(compose.activity)
        var allowed = false
        var denied = false
        compose.setContent { NolaTheme {
            PermissionDialog(toolName = "bash", message = "Details unavailable", executionLocation = "Host", dataBoundary = "Owned test", validity = "One operation",
                allowLabel = "Allow fixture once", conversationAllowLabel = "Allow fixture conversation", onAllowConversation = { allowed = true },
                onAllow = { allowed = true }, onDeny = { denied = true }, allowEnabled = false)
        } }
        compose.onNodeWithText("Allow fixture once").assertIsNotEnabled()
        compose.onNodeWithText("Allow fixture conversation").assertIsNotEnabled()
        compose.onNodeWithText(compose.activity.getString(R.string.common_deny)).assertIsEnabled().performClick()
        compose.runOnIdle { assertTrue(denied); assertFalse(allowed) }
    }
}
