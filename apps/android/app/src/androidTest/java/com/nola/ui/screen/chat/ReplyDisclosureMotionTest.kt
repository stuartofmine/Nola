// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.domain.projection.ConversationProjectionV1
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ReplyDisclosureMotionTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun canonicalProcessExpandsAndClosesThroughIntermediateHeights() {
        val body = "A stable reasoning body.\n".repeat(8)
        val part = ConversationProjectionV1.Part.Reasoning("motion-part", 1,
            ConversationProjectionV1.ActivityState.Completed, body)
        val choices = mutableStateOf(CanonicalActivityDisclosureChoices())
        compose.setContent { MaterialTheme {
            Column(Modifier.fillMaxWidth().testTag("motion-root")) {
                CanonicalActivityCards(listOf(part), false, choices.value, { choices.value = it }, {})
            }
        } }
        compose.waitForIdle()
        val heading = compose.activity.getString(com.nola.R.string.thinking_process)
        val closed = compose.onNodeWithTag("motion-root").fetchSemanticsNode().boundsInRoot.height
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText(heading).performClick()
        compose.mainClock.advanceTimeBy(64)
        val middle = compose.onNodeWithTag("motion-root").fetchSemanticsNode().boundsInRoot.height
        compose.mainClock.advanceTimeBy(240)
        val opened = compose.onNodeWithTag("motion-root").fetchSemanticsNode().boundsInRoot.height
        assertTrue(opened > closed)
        // System Reduce Motion may legitimately remove intermediate frames.
        val reduced = com.nola.ui.components.animatorDurationScaleDisablesMotion(
            android.provider.Settings.Global.getFloat(compose.activity.contentResolver,
                android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f))
        if (!reduced) assertTrue("Body height must interpolate", middle > closed && middle < opened)
        compose.onNodeWithText(heading).performClick()
        compose.mainClock.advanceTimeBy(48)
        compose.onNodeWithText(heading).performClick()
        compose.mainClock.advanceTimeBy(48)
        compose.onNodeWithText(heading).performClick()
        compose.mainClock.advanceTimeBy(260)
        assertEquals(closed, compose.onNodeWithTag("motion-root").fetchSemanticsNode().boundsInRoot.height, 1f)
        compose.mainClock.autoAdvance = true
    }
}
