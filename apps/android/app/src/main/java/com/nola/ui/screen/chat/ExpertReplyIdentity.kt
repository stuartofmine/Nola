// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.rememberTextMeasurer

internal val ExpertReplyStreamingKey = SemanticsPropertyKey<Boolean>("ExpertReplyStreaming")
internal var SemanticsPropertyReceiver.expertReplyStreaming by ExpertReplyStreamingKey
internal val ExpertReplyThinkingLengthKey = SemanticsPropertyKey<Int>("ExpertReplyThinkingLength")
internal var SemanticsPropertyReceiver.expertReplyThinkingLength by ExpertReplyThinkingLengthKey

internal fun expertInitials(name: String): String = name.trim().split(Regex("\\s+"))
    .filter { it.isNotBlank() }.take(2).joinToString("") { word ->
        String(Character.toChars(word.codePointAt(0)))
    }.uppercase(java.util.Locale.ROOT).ifBlank { "?" }

/** Decorative initials; the adjacent full name supplies the accessible identity. */
@Composable
internal fun ExpertReplyAvatar(name: String) {
    Surface(shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.size(30.dp).clearAndSetSemantics { }) {
        Box(contentAlignment = Alignment.Center) {
            val initial = expertInitials(name).let { String(Character.toChars(it.codePointAt(0))) }
            Text(initial, style = MaterialTheme.typography.labelSmall,
                maxLines = 1, softWrap = false, color = MaterialTheme.colorScheme.onSurface)
        }
    }
}

/** Name and avatar occupy the same outside column; neither consumes bubble space. */
@Composable
internal fun ExpertReplyIdentity(name: String) {
    Column(Modifier.width(if (LocalConfiguration.current.screenWidthDp < 360) 34.dp else 38.dp).padding(top = 3.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp)) {
        ExpertReplyAvatar(name)
        Text(name, style = MaterialTheme.typography.labelSmall.copy(fontSize = 12.sp, lineHeight = 18.sp),
            color = MaterialTheme.colorScheme.onSurface, textAlign = TextAlign.Center)
    }
}

/** Bound only short plain prose. Rich Markdown and activity surfaces retain their full layout width. */
@Composable
internal fun expertReplyBubbleWidth(text: String, horizontalPadding: Dp): Dp? {
    if (text.isBlank() || text.length > 512 || text.any { it in "\n\r`*_#>|[]{}$" } ||
        text.startsWith("-") || text.startsWith("+") || Regex("^\\d+[.)]\\s").containsMatchIn(text)) return null
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val measured = measurer.measure(text, style = MaterialTheme.typography.bodyLarge,
        softWrap = false, maxLines = 1)
    return with(density) { measured.size.width.toDp() } + horizontalPadding * 2 + 4.dp
}
