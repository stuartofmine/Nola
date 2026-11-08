// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.ui.theme.NolaTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class MessageCopyActionInteractionTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun compactCopyKeepsSeparateTouchTargetsAndAnnouncesItsResult() {
        requireIsolatedProductTestUser(compose.activity)
        var dark by mutableStateOf(false)
        var density = 1f
        compose.setContent {
            density = LocalDensity.current.density
            NolaTheme(darkTheme = dark) {
                Row(Modifier.width(240.dp)) {
                    MessageCopyAction("Owned clipboard acceptance", Modifier.testTag("copy"), compact = true)
                    TextButton(onClick = {}, modifier = Modifier.testTag("edit")) { Text("Edit and resend") }
                }
            }
        }
        for (appearance in listOf(false, true)) {
            compose.runOnIdle { dark = appearance }
            val copy = compose.onNodeWithTag("copy")
            copy.assertHasClickAction()
            val bounds = copy.fetchSemanticsNode().boundsInRoot
            val edit = compose.onNodeWithTag("edit").fetchSemanticsNode().boundsInRoot
            assertTrue(bounds.width >= 48 * density && bounds.height >= 48 * density)
            assertTrue(bounds.width <= 80 * density)
            assertTrue(bounds.right <= edit.left)
            // Sample feedback before Compose's automatic clock advances through
            // the 1.8-second dismissal coroutine.
            compose.mainClock.autoAdvance = false
            try {
                copy.performClick()
                compose.mainClock.advanceTimeByFrame()
                copy.assertContentDescriptionEquals(compose.activity.getString(R.string.chat_message_copied))
            } finally {
                compose.mainClock.autoAdvance = true
            }
        }
    }
}
