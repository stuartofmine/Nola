// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Exercises the actual native text view, rather than only the source semantics. */
@RunWith(AndroidJUnit4::class)
class StreamingParagraphLayoutTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun sentenceAppendsAndCompletionKeepOneNativeParagraph() {
        val source = mutableStateOf("A continuous paragraph starts here")
        val streaming = mutableStateOf(true)
        compose.setContent { MaterialTheme { StreamingText(source.value, streaming.value) } }
        compose.waitForIdle()
        lateinit var original: TextView
        fun paragraph(): TextView = textViews(compose.activity.window.decorView)
            .filter { it.text.toString().startsWith("A continuous paragraph") }.single()
        compose.runOnIdle { original = paragraph() }
        for (delta in listOf(", with a comma", ". The next sentence follows.", " 中文内容继续追加。", " **Bold text** remains in this paragraph.")) {
            compose.runOnIdle { source.value += delta }
            compose.waitForIdle()
            compose.runOnIdle { assertSame(original, paragraph()) }
        }
        compose.runOnIdle { streaming.value = false }
        compose.waitForIdle()
        compose.runOnIdle {
            assertSame(original, paragraph())
            assertEquals(source.value.replace("**", ""), original.text.toString())
        }
    }

    @Test
    fun waterWaveChangesOnlyNewInkAndCompletionSettlesIt() {
        val prefix = "A continuous paragraph starts here."
        val source = mutableStateOf(prefix)
        val streaming = mutableStateOf(true)
        compose.setContent { MaterialTheme { StreamingText(source.value, streaming.value) } }
        compose.waitForIdle()
        lateinit var paragraph: TextView
        compose.runOnIdle {
            paragraph = textViews(compose.activity.window.decorView)
                .single { it.text.toString().startsWith(prefix) }
        }
        compose.waitUntil(2_000) { waterSpans(paragraph).isEmpty() }
        val suffix = " New text 👨‍👩‍👧‍👦 é 🇨🇳 👍🏽 floats into clarity."
        compose.runOnIdle { source.value += suffix }
        compose.waitUntil(2_000) { paragraph.text.toString() == prefix + suffix && waterSpans(paragraph).isNotEmpty() }
        compose.runOnIdle {
            assertEquals(prefix + suffix, paragraph.text.toString())
            val spanned = paragraph.text as android.text.Spanned
            val boundaries = android.icu.text.BreakIterator.getCharacterInstance(java.util.Locale.ROOT)
            boundaries.setText(paragraph.text.toString())
            var hasActiveInk = false
            for (span in waterSpans(paragraph)) {
                val from = spanned.getSpanStart(span)
                val to = spanned.getSpanEnd(span)
                org.junit.Assert.assertTrue("Only new ink is animated", from >= prefix.length)
                org.junit.Assert.assertTrue(boundaries.isBoundary(from) && boundaries.isBoundary(to))
                val paint = android.text.TextPaint(paragraph.paint)
                span.updateDrawState(paint)
                if (paint.alpha < paragraph.paint.alpha) {
                    hasActiveInk = true
                    org.junit.Assert.assertTrue(paint.baselineShift >= paragraph.paint.baselineShift)
                }
            }
            org.junit.Assert.assertTrue("Actual native spans must paint an active water front", hasActiveInk)
            streaming.value = false
        }
        compose.waitUntil(2_000) { waterSpans(paragraph).isEmpty() }
        compose.runOnIdle { assertEquals(prefix + suffix, paragraph.text.toString()) }
    }

    private fun waterSpans(view: TextView): List<android.text.style.CharacterStyle> {
        val text = view.text as? android.text.Spanned ?: return emptyList()
        return text.getSpans(0, text.length, android.text.style.CharacterStyle::class.java)
            .filter { it.javaClass.simpleName == "WaterInk" }
    }

    private fun textViews(view: View): List<TextView> = when (view) {
        is TextView -> listOf(view)
        is ViewGroup -> (0 until view.childCount).flatMap { textViews(view.getChildAt(it)) }
        else -> emptyList()
    }
}
