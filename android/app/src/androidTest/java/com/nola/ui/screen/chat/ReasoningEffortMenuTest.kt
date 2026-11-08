// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.nola.ui.theme.NolaTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class ReasoningEffortMenuTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun fiveEffortsStayDistinctAndUnsupportedControlsAreHidden() {
        val efforts = listOf("none", "low", "medium", "high", "xhigh")
        var selected by mutableStateOf("automatic")
        var supported by mutableStateOf(true)
        compose.setContent { NolaTheme {
            ReasoningEffortMenu(if (supported) efforts else emptyList(), selected, true) { selected = it }
        } }
        for (effort in efforts + "automatic") {
            compose.onNodeWithTag("chat.composer.reasoning").performClick()
            compose.onNodeWithTag("chat.reasoning.$effort").performClick()
            compose.runOnIdle { assertEquals(effort, selected) }
        }
        compose.runOnIdle { supported = false }
        compose.onNodeWithTag("chat.composer.reasoning").assertDoesNotExist()
    }
}
