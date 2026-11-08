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

    private fun textViews(view: View): List<TextView> = when (view) {
        is TextView -> listOf(view)
        is ViewGroup -> (0 until view.childCount).flatMap { textViews(view.getChildAt(it)) }
        else -> emptyList()
    }
}
