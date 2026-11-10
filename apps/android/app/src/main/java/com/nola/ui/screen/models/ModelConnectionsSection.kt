package com.nola.ui.screen.models

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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

@Composable
internal fun ModelConnectionsSection(state: ModelsUiState, refresh: () -> Unit, manageHost: () -> Unit) {
    var showAccountDetails by remember { mutableStateOf(false) }
    NolaCardCompact {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(stringResource(R.string.model_connections_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            TextButton(enabled = !state.connectionInventoryLoading, onClick = refresh) { Text(stringResource(R.string.model_connections_refresh)) }
        }
        if (state.connectionInventoryLoading) Text(stringResource(R.string.model_connections_loading))
        if (state.connectionInventoryFailed) Text(stringResource(R.string.model_connections_failed), color = MaterialTheme.colorScheme.error)
        Text(stringResource(R.string.model_connections_host_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(enabled = !state.openingHostManagement && state.webHostUrl.isNotBlank() && state.webHostToken.isNotBlank(), onClick = manageHost) {
            Text(stringResource(R.string.model_connections_host_manage))
        }
        if (state.webHostUrl.isBlank() || state.webHostToken.isBlank()) {
            Text(stringResource(R.string.model_connections_pair_first), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        TextButton(onClick = { showAccountDetails = !showAccountDetails }) {
            Text(stringResource(if (showAccountDetails) R.string.canonical_hide_details else R.string.model_connections_account_details))
        }
        AnimatedVisibility(visible = showAccountDetails) {
            Text(
                stringResource(R.string.model_connections_source_boundary),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        state.connectionInventory?.let { inventory ->
            if (inventory.connections.isEmpty()) Text(stringResource(R.string.model_connections_empty), color = MaterialTheme.colorScheme.onSurfaceVariant)
            inventory.connections.forEach { connection ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(connection.name, style = MaterialTheme.typography.titleSmall)
                    Text(connection.address, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(connection.source, style = MaterialTheme.typography.labelMedium)
                    inventory.accounts.find { it.id == connection.accountId }?.let { account ->
                        Text("${account.name} · ${account.state}", style = MaterialTheme.typography.bodyMedium)
                    }
                    connection.models.forEach { name -> Text(name, style = MaterialTheme.typography.bodyMedium) }
                }
            }
        }
    }
}
