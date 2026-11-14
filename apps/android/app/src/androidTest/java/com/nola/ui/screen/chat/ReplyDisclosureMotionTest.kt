// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
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

    @Test fun inlineReferencesInProseAndTablesUseCaptionColorAndKeepTargets() {
        val dark = mutableStateOf(false)
        val expectedColor = java.util.concurrent.atomic.AtomicInteger()
        val source = "正文内联引用 [来源](https://example.com/inline)\n\n| 时间 | 说明 |\n| --- | --- |\n| 阶段一 | 保留正文说明及[来源](https://example.com/table) |"
        compose.setContent { MaterialTheme(colorScheme = if (dark.value) darkColorScheme() else lightColorScheme()) {
            expectedColor.set(MaterialTheme.colorScheme.onSurfaceVariant.toArgb())
            Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.background).padding(16.dp)) {
                com.nola.ui.components.MarkdownRenderer(source)
            }
        } }
        fun textViews(view: android.view.View): List<android.widget.TextView> =
            (if (view is android.widget.TextView) listOf(view) else emptyList()) +
                (if (view is android.view.ViewGroup) (0 until view.childCount).flatMap { textViews(view.getChildAt(it)) } else emptyList())
        for (isDark in listOf(false, true)) {
            compose.runOnIdle { dark.value = isDark }; compose.waitForIdle()
            compose.runOnIdle {
                for ((marker, target) in listOf("正文内联引用" to "https://example.com/inline", "保留正文说明" to "https://example.com/table")) {
                    val view = textViews(compose.activity.window.decorView).first { it.text.toString().contains(marker) }
                    val text = view.text as android.text.Spanned
                    assertTrue(!text.toString().contains("[来源]"))
                    val link = text.getSpans(0, text.length, io.noties.markwon.core.spans.LinkSpan::class.java).first()
                    assertEquals(target, link.url)
                    val paint = android.text.TextPaint(view.paint); link.updateDrawState(paint)
                    assertEquals(expectedColor.get(), paint.color)
                    val sizes = text.getSpans(text.getSpanStart(link), text.getSpanEnd(link), android.text.style.AbsoluteSizeSpan::class.java)
                    assertTrue(sizes.isNotEmpty()); assertTrue(sizes.first().size < view.textSize)
                }
            }
            val image = compose.onRoot().captureToImage().asAndroidBitmap()
            compose.activity.cacheDir.resolve("nola-inline-reference-${if (isDark) "dark" else "light"}.png").outputStream().use {
                image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
        }
    }

    @Test fun activeProcessKeepsItsTitleWhileExpandedAndBetweenSteps() {
        val live = mutableStateOf(true)
        val part = ConversationProjectionV1.Part.Reasoning("settled", 1, ConversationProjectionV1.ActivityState.Completed, "Completed step")
        val choices = mutableStateOf(CanonicalActivityDisclosureChoices())
        compose.setContent { MaterialTheme { CanonicalActivityCards(listOf(part), live.value, choices.value, { choices.value = it }, {}) } }
        val processing = compose.activity.getString(com.nola.R.string.chat_processing)
        compose.onNodeWithText(processing).performClick()
        compose.onNodeWithText(processing).assertExists()
        compose.onNodeWithText("Completed step").assertExists()
        compose.runOnIdle { live.value = false }
        compose.onNodeWithText(compose.activity.getString(com.nola.R.string.thinking_process)).assertExists()
    }

    @Test fun referencesStartWithThreeRowsAndExpandWithoutTechnicalCards() {
        val parts = (1..31).map { index -> ConversationProjectionV1.Part.Source("s$index", 1, "s$index", "Reference $index", "https://example.com/$index", null, null) }
        val dark = mutableStateOf(false)
        compose.setContent { MaterialTheme(colorScheme = if (dark.value) darkColorScheme() else lightColorScheme()) {
            Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.background).verticalScroll(rememberScrollState()).padding(16.dp)) { CanonicalSourceCards(parts, {}) }
        } }
        for (isDark in listOf(false, true)) {
            compose.runOnIdle { dark.value = isDark }
            compose.waitForIdle()
            val image = compose.onRoot().captureToImage().asAndroidBitmap()
            val file = compose.activity.cacheDir.resolve("nola-cross-reference-${if (isDark) "dark" else "light"}.png")
            file.outputStream().use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        }
        compose.onNodeWithText("[3] Reference 3").assertExists()
        compose.onNodeWithText("[4] Reference 4").assertDoesNotExist()
        compose.onNodeWithTag("chat.sources.toggle").performScrollTo().performClick()
        compose.onNodeWithText("[31] Reference 31").assertExists()
        compose.onNodeWithTag("chat.sources.toggle").performScrollTo().performClick()
        compose.onNodeWithText("[4] Reference 4").assertDoesNotExist()
    }

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
