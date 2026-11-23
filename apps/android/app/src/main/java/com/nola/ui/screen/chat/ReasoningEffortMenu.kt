// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.ui.components.NolaDisclosureChevron
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.rememberReduceMotionEnabled

@Composable internal fun reasoningEffortLabel(value: String): String = when (value) {
    "automatic" -> stringResource(R.string.reasoning_effort_automatic)
    "none" -> stringResource(R.string.reasoning_effort_none)
    "minimal" -> stringResource(R.string.reasoning_effort_minimal)
    "low" -> stringResource(R.string.reasoning_effort_low)
    "medium" -> stringResource(R.string.reasoning_effort_medium)
    "high" -> stringResource(R.string.reasoning_effort_high)
    "xhigh" -> stringResource(R.string.reasoning_effort_xhigh)
    "max" -> stringResource(R.string.reasoning_effort_max)
    "concise" -> stringResource(R.string.models_reasoning_simple)
    "balanced" -> stringResource(R.string.models_reasoning_medium)
    "detailed" -> stringResource(R.string.models_reasoning_complex)
    else -> value
}

@Composable internal fun ReasoningEffortMenu(efforts: List<String>, preference: String,
    enabled: Boolean, onSelect: (String) -> Unit) {
    if (efforts.isEmpty()) return
    val menuEfforts = remember(efforts) { (listOf("automatic") + efforts).distinct() }
    val reduceMotion = rememberReduceMotionEnabled()
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            modifier = Modifier
                .heightIn(min = 48.dp)
                .clip(RoundedCornerShape(NolaRadius.pill))
                .clickable(
                    enabled = enabled,
                    role = Role.DropdownList,
                    onClickLabel = stringResource(R.string.reasoning_effort_title),
                    onClick = { open = true }
                )
                .testTag("chat.composer.reasoning")
                .padding(horizontal = 10.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Text(
                reasoningEffortLabel(preference),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.labelMedium,
                color = if (enabled) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f)
            )
            NolaDisclosureChevron(expanded = open, reduceMotion = reduceMotion)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            Text(stringResource(R.string.reasoning_effort_title), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.labelMedium)
            for (effort in menuEfforts) {
                DropdownMenuItem(text = { Text(reasoningEffortLabel(effort)) },
                    trailingIcon = { if (effort == preference) Icon(Icons.Filled.Check, null) },
                    onClick = { open = false; onSelect(effort) }, enabled = enabled,
                    modifier = Modifier.testTag("chat.reasoning.$effort"))
            }
        }
    }
}
