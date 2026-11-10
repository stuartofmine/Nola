// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R

@Composable internal fun reasoningEffortLabel(value: String): String = when (value) {
    "automatic" -> stringResource(R.string.reasoning_effort_automatic)
    "none" -> stringResource(R.string.reasoning_effort_none)
    "minimal" -> stringResource(R.string.reasoning_effort_minimal)
    "low" -> stringResource(R.string.reasoning_effort_low)
    "medium" -> stringResource(R.string.reasoning_effort_medium)
    "high" -> stringResource(R.string.reasoning_effort_high)
    "xhigh" -> stringResource(R.string.reasoning_effort_xhigh)
    "max" -> stringResource(R.string.reasoning_effort_max)
    else -> value
}

@Composable internal fun ReasoningEffortMenu(efforts: List<String>, preference: String,
    enabled: Boolean, onSelect: (String) -> Unit) {
    if (efforts.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, enabled = enabled,
            modifier = Modifier.testTag("chat.composer.reasoning")) {
            Text(reasoningEffortLabel(preference), maxLines = 1, style = MaterialTheme.typography.labelMedium)
            Spacer(Modifier.width(4.dp))
            Icon(Icons.Filled.ExpandMore, contentDescription = stringResource(R.string.reasoning_effort_title), modifier = Modifier.size(14.dp))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            Text(stringResource(R.string.reasoning_effort_title), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.labelMedium)
            for (effort in listOf("automatic") + efforts) {
                DropdownMenuItem(text = { Text(reasoningEffortLabel(effort)) },
                    trailingIcon = { if (effort == preference) Icon(Icons.Filled.Check, null) },
                    onClick = { open = false; onSelect(effort) }, enabled = enabled,
                    modifier = Modifier.testTag("chat.reasoning.$effort"))
            }
        }
    }
}
