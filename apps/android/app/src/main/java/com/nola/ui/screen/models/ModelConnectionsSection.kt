package com.nola.ui.screen.models

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import com.nola.ui.components.NolaDisclosureBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.ui.components.NolaCardCompact
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberReduceMotionEnabled

@Composable
internal fun ModelConnectionsSection(
    state: ModelsUiState,
    refresh: () -> Unit,
    connectHost: () -> Unit,
    manageHost: () -> Unit,
) {
    val reduceMotion = rememberReduceMotionEnabled()
    var showAccountDetails by remember { mutableStateOf(false) }
    NolaCardCompact {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(stringResource(R.string.model_connections_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            TextButton(enabled = !state.connectionInventoryLoading, onClick = refresh) { Text(stringResource(R.string.model_connections_refresh)) }
        }
        if (state.connectionInventoryLoading) Text(stringResource(R.string.model_connections_loading))
        if (state.connectionInventoryFailed) Text(stringResource(R.string.model_connections_failed), color = MaterialTheme.colorScheme.error)
        Text(stringResource(R.string.model_connections_host_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (state.webHostUrl.isBlank() || state.webHostToken.isBlank()) {
            TextButton(onClick = connectHost) { Text(stringResource(R.string.onboarding_guide_runtime_host)) }
        } else {
            TextButton(enabled = !state.openingHostManagement, onClick = manageHost) {
                Text(stringResource(R.string.model_connections_host_manage))
            }
        }
        NolaDisclosureBar(
            title = stringResource(
                if (showAccountDetails) R.string.canonical_hide_details
                else R.string.model_connections_account_details
            ),
            expanded = showAccountDetails,
            onToggle = { showAccountDetails = !showAccountDetails }
        )
        AnimatedVisibility(
            visible = showAccountDetails,
            enter = nolaDisclosureEnter(reduceMotion),
            exit = nolaDisclosureExit(reduceMotion),
        ) {
            Text(
                stringResource(R.string.model_connections_source_boundary),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        state.connectionInventory?.let { inventory ->
            if (inventory.connections.isEmpty()) Text(stringResource(R.string.model_connections_empty), color = MaterialTheme.colorScheme.onSurfaceVariant)
            inventory.connections.forEach { connection ->
                val sourceLabel = when (connection.source.lowercase()) {
                    "open_code", "opencode" -> "OpenCode"
                    "claude_code" -> "Claude Code"
                    "codex" -> "Codex"
                    else -> connection.source.replace('_', ' ').replaceFirstChar { it.uppercase() }
                }
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(connection.name, style = MaterialTheme.typography.titleSmall)
                    Text(connection.address, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(sourceLabel, style = MaterialTheme.typography.labelMedium)
                    inventory.accounts.find { it.id == connection.accountId }?.let { account ->
                        val accountState = account.state.replace('_', ' ').replaceFirstChar { it.uppercase() }
                        Text("${account.name} · $accountState", style = MaterialTheme.typography.bodyMedium)
                    }
                    connection.models.forEach { name -> Text(name, style = MaterialTheme.typography.bodyMedium) }
                }
            }
        }
    }
}
