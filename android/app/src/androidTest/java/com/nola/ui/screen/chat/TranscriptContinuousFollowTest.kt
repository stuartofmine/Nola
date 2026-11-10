// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import org.junit.Assert.assertEquals
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.ProcessingState
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** A real native transcript must follow during a stream, not only after EOF. */
@RunWith(AndroidJUnit4::class)
class TranscriptContinuousFollowTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun continuousDeltasFollowCommittedGeometryBeforeTheStreamEnds() {
        val response = ConversationMessage(id = "answer", conversationId = "fixture", role = ChatRole.assistant,
            text = "A continuous answer begins.\n")
        val ui = mutableStateOf(ChatUiState(conversationId = "fixture", messages = listOf(response),
            activeResponseId = response.id, currentResponse = response.text, processingState = ProcessingState.Streaming))
        lateinit var viewport: LazyListState
        compose.setContent {
            val list = rememberLazyListState()
            SideEffect { viewport = list }
            MaterialTheme { ChatTranscript(state = ui.value, listState = list, modifier = Modifier.height(360.dp)) }
        }
        compose.waitForIdle()
        // Use the native event loop, keeping deltas faster than the old 48 ms
        // cancellation window. Do not wait for Compose idle between appends.
        var accepted = response.text
        val followedWhileLive = java.util.concurrent.atomic.AtomicBoolean(false)
        repeat(100) { index ->
            accepted += "Line $index advances without restarting follow. 中文继续。\n"
            val text = accepted
            compose.activity.runOnUiThread {
                ui.value = ui.value.copy(currentResponse = text, messages = listOf(response.copy(text = text)))
                if (index > 20 && !viewport.canScrollForward) followedWhileLive.set(true)
            }
            android.os.SystemClock.sleep(24)
        }
        assertTrue("Viewport must make progress before an uninterrupted stream ends", followedWhileLive.get())
        compose.waitUntil(4_000) { !viewport.canScrollForward }
        compose.runOnIdle {
            assertTrue(ui.value.processingState.isGenerating)
            assertTrue(ui.value.messages.last().text.endsWith("中文继续。\n"))
        }
        compose.onNodeWithTag("chat-transcript-list").performTouchInput { swipeDown() }
        compose.waitForIdle()
        var historyOffset = 0
        compose.runOnIdle {
            historyOffset = viewport.firstVisibleItemScrollOffset
            assertTrue("A gesture must move the reader away from the tail: index=${viewport.firstVisibleItemIndex}, offset=$historyOffset, items=${viewport.layoutInfo.totalItemsCount}", viewport.canScrollForward)
        }
        compose.runOnIdle { ui.value = ui.value.copy(currentResponse = accepted + "New tail after user scroll.\n".repeat(20)) }
        android.os.SystemClock.sleep(300)
        compose.waitForIdle()
        compose.runOnIdle { assertEquals("New content cannot undo the reader's gesture", historyOffset,
            viewport.firstVisibleItemScrollOffset) }
        compose.onNodeWithTag("chat.transcript.followLatest").performClick()
        compose.waitUntil(4_000) { !viewport.canScrollForward }
        compose.onNodeWithTag("chat-transcript-list").performTouchInput { swipeDown() }
        compose.waitForIdle()
        repeat(6) {
            if (viewport.canScrollForward) {
                compose.onNodeWithTag("chat-transcript-list").performTouchInput { swipeUp() }
                compose.waitForIdle()
            }
        }
        compose.waitUntil(4_000) { !viewport.canScrollForward }
        val oldHeight = viewport.layoutInfo.visibleItemsInfo.firstOrNull { it.index == 0 }?.size ?: 0
        compose.runOnIdle { ui.value = ui.value.copy(currentResponse = ui.value.currentResponse + "Resumed tail.\n".repeat(30)) }
        compose.waitUntil(4_000) { (viewport.layoutInfo.visibleItemsInfo.firstOrNull { it.index == 0 }?.size ?: 0) > oldHeight }
        compose.waitUntil(4_000) { !viewport.canScrollForward }

    }
}
