package com.nola.ui.screen.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.data.remote.dto.ExecutionBackendAvailability
import com.nola.data.remote.dto.ExecutionBackendDto

internal const val CHAT_EXECUTION_BACKEND_PICKER_TEST_TAG = "chat_execution_backend_picker"
internal const val CHAT_EXECUTION_BACKEND_OPTION_TEST_TAG_PREFIX = "chat_execution_backend_option."

@Composable
internal fun ExecutionBackendSelector(
    backends: List<ExecutionBackendDto>,
    selectedBackendId: String,
    expanded: Boolean,
    onExpandedChange: (Boolean) -> Unit,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (backends.isEmpty()) return
    val selectedBackend = backends.firstOrNull { it.id == selectedBackendId }
        ?: backends.firstOrNull { it.id == "nola" }
        ?: backends.first()

    Box(modifier) {
        Row(
            modifier = Modifier
                .widthIn(max = 132.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.10f))
                .border(0.5.dp, MaterialTheme.colorScheme.primary.copy(alpha = 0.18f), CircleShape)
                .clickable(role = androidx.compose.ui.semantics.Role.Button) { onExpandedChange(true) }
                .heightIn(min = 48.dp)
                .padding(horizontal = 9.dp, vertical = 7.dp)
                .testTag(CHAT_EXECUTION_BACKEND_PICKER_TEST_TAG),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Icon(
                Icons.Filled.Computer,
                contentDescription = stringResource(R.string.chat_execution_unit),
                modifier = Modifier.size(15.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
            Text(
                text = selectedBackend.displayName,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.primary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Icon(
                Icons.Filled.ExpandMore,
                contentDescription = null,
                modifier = Modifier.size(13.dp),
                tint = MaterialTheme.colorScheme.primary.copy(alpha = 0.72f),
            )
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { onExpandedChange(false) },
        ) {
            Text(stringResource(R.string.execution_scope_host), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall)
            Text(stringResource(R.string.execution_scope_child), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall)
            if (!selectedBackend.capabilities.supportsPermissionBridge) {
                Text(stringResource(R.string.execution_scope_no_approval_bridge), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall)
            }
            backends.forEach { backend ->
                val availability = backend.availabilityState
                DropdownMenuItem(
                    modifier = Modifier.testTag(CHAT_EXECUTION_BACKEND_OPTION_TEST_TAG_PREFIX + backend.id),
                    text = {
                        Column {
                            Text(backend.displayName, style = MaterialTheme.typography.bodyMedium)
                            val detail = when {
                                availability == ExecutionBackendAvailability.Available && !backend.version.isNullOrBlank() -> backend.version
                                availability == ExecutionBackendAvailability.NotInstalled -> stringResource(R.string.chat_execution_unit_not_installed)
                                availability == ExecutionBackendAvailability.SignInRequired -> stringResource(R.string.chat_execution_unit_sign_in_required)
                                availability == ExecutionBackendAvailability.Incompatible -> stringResource(R.string.chat_execution_unit_incompatible)
                                availability != ExecutionBackendAvailability.Available -> stringResource(R.string.chat_execution_unit_unavailable)
                                else -> null
                            }
                            if (!detail.isNullOrBlank()) {
                                Text(
                                    detail,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    },
                    trailingIcon = {
                        if (backend.id == selectedBackendId) {
                            Icon(Icons.Filled.Check, null, Modifier.size(16.dp))
                        }
                    },
                    enabled = availability == ExecutionBackendAvailability.Available,
                    onClick = { onSelect(backend.id) },
                )
            }
        }
    }
}
