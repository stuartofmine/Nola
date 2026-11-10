// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.content.SharedPreferences
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.nola.R

/** Device presentation preference only; it carries no model or execution configuration. */
internal enum class ExpertBubbleColor(val key: String, val label: Int, private val light: Long, private val dark: Long) {
    Red("red", R.string.expert_color_red, 0xFFF8F1F1, 0xFF1C1313),
    Orange("orange", R.string.expert_color_orange, 0xFFF8F4EF, 0xFF1C1712),
    Yellow("yellow", R.string.expert_color_yellow, 0xFFF7F6ED, 0xFF1A190F),
    Green("green", R.string.expert_color_green, 0xFFF1F6F2, 0xFF111A14),
    Teal("teal", R.string.expert_color_teal, 0xFFEFF6F6, 0xFF101A1B),
    Blue("blue", R.string.expert_color_blue, 0xFFF2F6F8, 0xFF11171B),
    Purple("purple", R.string.expert_color_purple, 0xFFF6F3F7, 0xFF18141C),
    Pink("pink", R.string.expert_color_pink, 0xFFF8F2F5, 0xFF1C1318);

    fun background(darkAppearance: Boolean): Color = Color(if (darkAppearance) dark else light)
    companion object {
        fun fromKey(key: String?): ExpertBubbleColor = entries.firstOrNull { it.key == key } ?: Blue
    }
}

private fun preferenceKey(agentId: String) = "agent.$agentId.bubble-color"

@Composable
internal fun rememberExpertBubbleColor(agentId: String): ExpertBubbleColor {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("nola.expert-presentation", android.content.Context.MODE_PRIVATE) }
    val key = remember(agentId) { preferenceKey(agentId) }
    var color by remember(preferences, key) { mutableStateOf(ExpertBubbleColor.fromKey(preferences.getString(key, null))) }
    DisposableEffect(preferences, key) {
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { source, changed ->
            if (changed == key) color = ExpertBubbleColor.fromKey(source.getString(key, null))
        }
        preferences.registerOnSharedPreferenceChangeListener(listener)
        onDispose { preferences.unregisterOnSharedPreferenceChangeListener(listener) }
    }
    return color
}

@Composable
internal fun ExpertBubbleColorEditor(agentId: String, name: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val saved = rememberExpertBubbleColor(agentId)
    var draft by remember(agentId) { mutableStateOf(saved) }
    var saveFailed by remember(agentId) { mutableStateOf(false) }
    val dark = MaterialTheme.colorScheme.background.luminance() < 0.5f
    AlertDialog(
        containerColor = MaterialTheme.colorScheme.background,
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.expert_color_title, name)) },
        text = {
            Column(Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState())) {
                Row(Modifier.fillMaxWidth().padding(bottom = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    ExpertReplyIdentity(name)
                    Text(stringResource(R.string.expert_color_preview), Modifier.weight(1f)
                        .background(draft.background(dark), RoundedCornerShape(topStart = 5.dp,
                            topEnd = 14.dp, bottomEnd = 14.dp, bottomStart = 14.dp)).padding(14.dp),
                        color = MaterialTheme.colorScheme.onSurface, style = MaterialTheme.typography.bodyLarge)
                }
                ExpertBubbleColor.entries.forEach { candidate ->
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp)
                        .testTag("expert.color.${candidate.key}")
                        .selectable(selected = candidate == draft, role = Role.RadioButton, onClick = { draft = candidate })
                        .padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        RadioButton(selected = candidate == draft, onClick = null)
                        Box(Modifier.size(28.dp).background(candidate.background(dark), CircleShape)
                            .border(0.5.dp, MaterialTheme.colorScheme.outlineVariant, CircleShape))
                        Text(stringResource(candidate.label), color = MaterialTheme.colorScheme.onSurface)
                    }
                }
                if (saveFailed) Text(stringResource(R.string.expert_color_save_failed), color = MaterialTheme.colorScheme.error)
            }
        },
        confirmButton = { TextButton(onClick = {
            val savedOk = context.getSharedPreferences("nola.expert-presentation", android.content.Context.MODE_PRIVATE)
                .edit().putString(preferenceKey(agentId), draft.key).commit()
            if (savedOk) onDismiss() else saveFailed = true
        }) { Text(stringResource(R.string.common_save)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel)) } },
    )
}
