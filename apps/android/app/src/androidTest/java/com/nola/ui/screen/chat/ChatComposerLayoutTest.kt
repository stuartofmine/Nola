// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.height
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.Alignment
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onParent
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.unit.dp
import com.nola.ui.theme.NolaTheme
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class ChatComposerLayoutTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun nativeConversationKeepsReadingWidthAcrossThemesAndFontSizes() {
        requireIsolatedProductTestUser(compose.activity)
        var width by mutableStateOf(320)
        var dark by mutableStateOf(false)
        var large by mutableStateOf(false)
        var draft by mutableStateOf("Draft stays here")
        var expectedBodySizePx = 0f
        compose.setContent {
            val density = androidx.compose.ui.platform.LocalDensity.current
            androidx.compose.runtime.CompositionLocalProvider(
                androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(density.density, if (large) 2f else 1f)
            ) {
                NolaTheme(darkTheme = dark) {
                    val scaledDensity = androidx.compose.ui.platform.LocalDensity.current
                    val bodyStyle = androidx.compose.material3.MaterialTheme.typography.bodyLarge
                    val bodySize = with(scaledDensity) { bodyStyle.fontSize.toPx() }
                    androidx.compose.runtime.SideEffect { expectedBodySizePx = bodySize }
                    androidx.compose.foundation.layout.Column(Modifier.width(width.dp).height(700.dp)
                        .background(androidx.compose.material3.MaterialTheme.colorScheme.background).testTag("mobile.fixture")) {
                        androidx.compose.foundation.layout.Column(Modifier.weight(1f).verticalScroll(androidx.compose.foundation.rememberScrollState())) {
                            MessageBubble(com.nola.domain.model.ConversationMessage(id = "user", conversationId = "fixture",
                                role = com.nola.domain.model.ChatRole.user, text = "周末想找个地方走走，不想把行程排得太满。"))
                            MessageBubble(com.nola.domain.model.ConversationMessage(id = "answer", conversationId = "fixture",
                                role = com.nola.domain.model.ChatRole.assistant,
                                text = "那就把目标定小一点：一段山路，一顿好吃的，再留一点空白。\n\n上午慢慢出发，在山脚吃午饭，下午选一条能随时折返的路线。"))
                        }
                        ChatComposerBar(inputText = draft, modelControl = { TextButton(onClick = {}) { Text("Qwen · 日常") } },
                            onInputChange = { draft = it }, attachments = emptyList(), attachmentMessage = null,
                            isImportingAttachments = false, enabled = true, isGenerating = false, draftWritable = true,
                            onAttach = {}, onPasteAttachments = { false }, onRemoveAttachment = {}, onSendOrStop = {})
                    }
                }
            }
        }
        for (w in listOf(320, 390)) for (d in listOf(false, true)) for (l in listOf(false, true)) {
            compose.runOnIdle { width = w; dark = d; large = l }
            compose.waitForIdle()
            val editor = compose.onNodeWithTag("chat.composer.input").fetchSemanticsNode().boundsInRoot
            val frame = compose.onNodeWithTag("mobile.fixture").fetchSemanticsNode().boundsInRoot
            assertTrue("The editor must keep its reading width", editor.width > frame.width * 0.80f)
            assertTrue("Input cannot overflow the phone", editor.right <= frame.right)
            compose.onNodeWithText("Draft stays here", useUnmergedTree = true).assertExists()
            compose.runOnIdle {
                fun textViews(view: android.view.View): List<android.widget.TextView> = when (view) {
                    is android.widget.TextView -> listOf(view)
                    is android.view.ViewGroup -> (0 until view.childCount).flatMap { textViews(view.getChildAt(it)) }
                    else -> emptyList()
                }
                val answer = textViews(compose.activity.window.decorView).first { it.text.contains("那就把目标") }
                assertEquals("Native prose must follow Compose font scaling", expectedBodySizePx, answer.textSize, 0.5f)
            }
            val bitmap = compose.onNodeWithTag("mobile.fixture").captureToImage().asAndroidBitmap()
            val dir = InstrumentationRegistry.getInstrumentation().targetContext.getExternalFilesDir(null)!!
            File(dir, "mobile-chat-$w-${if (d) "dark" else "light"}-${if (l) "large" else "standard"}.png").outputStream().use {
                bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
        }
    }

    @OptIn(ExperimentalTestApi::class)
    @Test fun attachmentPasteDoesNotHijackModifiedKeyboardShortcuts() {
        requireIsolatedProductTestUser(compose.activity)
        var pastes = 0
        compose.setContent {
            NolaTheme {
                ChatComposerBar(Modifier.width(320.dp), inputText = "draft", modelControl = {},
                    onInputChange = {}, attachments = emptyList(), attachmentMessage = null,
                    isImportingAttachments = false, enabled = true, isGenerating = false,
                    draftWritable = true, onAttach = {}, onPasteAttachments = { pastes++; true },
                    onRemoveAttachment = {}, onSendOrStop = {})
            }
        }
        val editor = compose.onNodeWithTag("chat.composer.input")
        editor.performClick()
        editor.performKeyInput {
            keyDown(Key.CtrlLeft); keyDown(Key.AltLeft); pressKey(Key.V)
            keyUp(Key.AltLeft); keyUp(Key.CtrlLeft)
            keyDown(Key.CtrlLeft); keyDown(Key.ShiftLeft); pressKey(Key.V)
            keyUp(Key.ShiftLeft); keyUp(Key.CtrlLeft)
        }
        compose.runOnIdle { assertEquals(0, pastes) }
        editor.performKeyInput { keyDown(Key.CtrlLeft); pressKey(Key.V); keyUp(Key.CtrlLeft) }
        compose.runOnIdle { assertEquals(1, pastes) }
    }

    @Test fun phoneToolbarFollowsInputAndKeepsSendAndStopReachable() {
        var text by mutableStateOf("")
        var generating by mutableStateOf(false)
        var writable by mutableStateOf(true)
        var dark by mutableStateOf(false)
        var reasoning by mutableStateOf("medium")
        var sends = 0
        var models = 0
        compose.setContent {
            NolaTheme(darkTheme = dark) {
                ChatComposerBar(Modifier.width(320.dp), inputText = text,
                    modelControl = { Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.weight(1f)) { TextButton(onClick = { models++ }, modifier = Modifier.testTag("fixture.model")) {
                            Text("A long model name", maxLines = 1)
                        } }
                        ReasoningEffortMenu(listOf("none", "low", "medium", "high", "xhigh"), reasoning, true) { reasoning = it }
                    } },
                    onInputChange = { text = it }, attachments = emptyList(), attachmentMessage = null,
                    isImportingAttachments = false, enabled = true, isGenerating = generating,
                    draftWritable = writable, onAttach = {}, onPasteAttachments = { false },
                    onRemoveAttachment = {}, onSendOrStop = { sends++ })
            }
        }
        val editor = compose.onNodeWithTag("chat.composer.input")
        val model = compose.onNodeWithTag("fixture.model")
        compose.waitForIdle()
        val editorBounds = editor.fetchSemanticsNode().boundsInRoot
        val modelBounds = model.fetchSemanticsNode().boundsInRoot
        assertTrue("Model control must follow the writing area", modelBounds.top >= editorBounds.bottom)
        assertTrue("Writing area should use the full container", editorBounds.width > modelBounds.width)
        model.performClick()
        assertEquals(1, models)
        editor.performTextInput("Hello")
        compose.onNodeWithTag("chat.composer.send", useUnmergedTree = true).onParent().performClick()
        assertEquals(1, sends)
        compose.runOnIdle { generating = true; writable = false }
        compose.onNodeWithTag("chat.composer.send", useUnmergedTree = true).onParent().performClick()
        assertEquals("Stopping must survive an unwritable draft", 2, sends)
        compose.runOnIdle { generating = false; writable = true; text = "" }
        for (scheme in listOf(false, true)) {
            compose.runOnIdle { dark = scheme }
            compose.waitForIdle()
            val bitmap = compose.onNodeWithTag("chat.composer").captureToImage().asAndroidBitmap()
            val dir = InstrumentationRegistry.getInstrumentation().targetContext.getExternalFilesDir(null)!!
            File(dir, "composer-${if (scheme) "dark" else "light"}.png").outputStream().use {
                bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
        }
    }
}
