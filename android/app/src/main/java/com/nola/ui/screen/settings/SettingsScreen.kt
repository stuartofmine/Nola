package com.nola.ui.screen.settings

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.ClipboardManager
import android.net.Uri
import android.provider.Settings
import android.text.format.Formatter
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.RadioButton
import com.nola.data.local.datastore.AppearancePreference
import com.nola.ui.components.NolaIconButton
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.CalendarMonth
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Dataset
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Extension
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.NotificationsOff
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.core.app.NotificationManagerCompat
import com.nola.R
import com.nola.ui.theme.nolaSurface
import com.nola.ui.theme.NolaContinuousShape
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaIconBadge
import com.nola.ui.components.NolaPage
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.screen.notifications.NotificationCenterScreen
import com.nola.ui.screen.scheduled.ScheduledActionsScreen
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onOpenDataSources: () -> Unit = {},
    onOpenModels: () -> Unit = {},
    onOpenIntelligence: () -> Unit = {},
    onOpenChat: () -> Unit = {}
) {
    var activeScreen by rememberSaveable { mutableStateOf<String?>(null) }

    BackHandler(enabled = activeScreen != null) {
        activeScreen = null
    }

    when (activeScreen) {
        "notifications" -> NotificationCenterScreen(onBack = { activeScreen = null })
        "scheduled" -> ScheduledActionsScreen(
            onBack = { activeScreen = null },
            onCreateInChat = onOpenChat
        )
        else -> SettingsMain(
            onNotifications = { activeScreen = "notifications" },
            onScheduledActions = { activeScreen = "scheduled" },
            onOpenDataSources = onOpenDataSources,
            onOpenModels = onOpenModels,
            onOpenIntelligence = onOpenIntelligence
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsMain(
    onNotifications: () -> Unit = {},
    onScheduledActions: () -> Unit = {},
    onOpenDataSources: () -> Unit = {},
    onOpenModels: () -> Unit = {},
    onOpenIntelligence: () -> Unit = {},
    viewModel: SettingsViewModel = hiltViewModel()
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val appearance by viewModel.appearance.collectAsStateWithLifecycle(initialValue = AppearancePreference.System)
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var showDeleteDialog by remember { mutableStateOf(false) }
    var showExportDialog by remember { mutableStateOf(false) }
    var detailSection by rememberSaveable { mutableStateOf<String?>(null) }
    BackHandler(enabled = detailSection != null) { detailSection = null }
    var isClearingData by remember { mutableStateOf(false) }
    var isExportingData by remember { mutableStateOf(false) }
    var operationMessage by rememberSaveable { mutableStateOf<String?>(null) }
    var operationFailed by rememberSaveable { mutableStateOf(false) }
    var notificationsAllowed by remember { mutableStateOf(NotificationManagerCompat.from(context).areNotificationsEnabled()) }
    val lifecycleOwner = LocalLifecycleOwner.current

    DisposableEffect(lifecycleOwner, context) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                notificationsAllowed = NotificationManagerCompat.from(context).areNotificationsEnabled()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    val sizeStr = when (val dataSize = state.localDataSizeBytes) {
        null -> stringResource(R.string.settings_size_calculating)
        else -> Formatter.formatFileSize(context, dataSize)
    }

    // Delete confirmation dialog
    if (showDeleteDialog) {
        AlertDialog(
            onDismissRequest = { if (!isClearingData) showDeleteDialog = false },
            title = { Text(stringResource(R.string.settings_clear_all_data)) },
            text = { Text(stringResource(R.string.settings_clear_all_data_message)) },
            confirmButton = {
                Button(
                    onClick = {
                        isClearingData = true
                        scope.launch {
                            try {
                                viewModel.clearAllData()
                                showDeleteDialog = false
                                operationFailed = false
                                operationMessage = context.getString(R.string.settings_all_data_cleared)
                            } catch (error: Exception) {
                                operationFailed = true
                                operationMessage = context.getString(R.string.settings_clear_failed, error.message ?: "")
                            } finally {
                                isClearingData = false
                            }
                        }
                    },
                    enabled = !isClearingData,
                    colors = ButtonDefaults.buttonColors(containerColor = NolaColors.danger)
                ) {
                    if (isClearingData) {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.settings_clearing_data))
                    } else {
                        Text(stringResource(R.string.settings_delete_all))
                    }
                }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteDialog = false }, enabled = !isClearingData) { Text(stringResource(R.string.common_cancel)) }
            }
        )
    }

    if (showExportDialog) {
        AlertDialog(
            onDismissRequest = { showExportDialog = false },
            title = { Text(stringResource(R.string.settings_export_confirm_title)) },
            text = { Text(stringResource(R.string.settings_export_confirm_message)) },
            confirmButton = {
                Button(
                    onClick = {
                        showExportDialog = false
                        isExportingData = true
                        scope.launch {
                            try {
                                val exportFile = viewModel.createExportArchive()
                                val uri = androidx.core.content.FileProvider.getUriForFile(
                                    context, "${context.packageName}.fileprovider", exportFile
                                )
                                val shareIntent = Intent(Intent.ACTION_SEND).apply {
                                    type = "application/zip"
                                    putExtra(Intent.EXTRA_STREAM, uri)
                                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                }
                                operationFailed = false
                                operationMessage = context.getString(R.string.settings_export_ready)
                                context.startActivity(Intent.createChooser(shareIntent, context.getString(R.string.settings_export_data)))
                            } catch (e: Exception) {
                                operationFailed = true
                                operationMessage = context.getString(R.string.settings_export_failed, e.message ?: "")
                            } finally {
                                isExportingData = false
                            }
                        }
                    }
                ) { Text(stringResource(R.string.settings_export_confirm_action)) }
            },
            dismissButton = {
                TextButton(onClick = { showExportDialog = false }) { Text(stringResource(R.string.common_cancel)) }
            }
        )
    }

    NolaPage {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            if (detailSection != null) NolaIconButton(label = stringResource(R.string.common_back), onClick = { detailSection = null }) {
                Icon(painterResource(R.drawable.nola_back), stringResource(R.string.common_back))
            }
            Text(stringResource(if (detailSection == "appearance") R.string.appearance_title else if (detailSection == "privacy") R.string.settings_privacy else if (detailSection == "notifications") R.string.notifications_title else R.string.settings_title),
                Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        }
        if (detailSection == null) {
            Column(Modifier.fillMaxWidth().nolaSurface(radius = 14.dp)) {
                IMSettingsRow(R.drawable.nola_models, R.string.models_title, onOpenModels)
                IMSettingsRow(R.drawable.nola_execution, R.string.settings_capabilities, onOpenIntelligence)
            }
            Column(Modifier.fillMaxWidth().nolaSurface(radius = 14.dp)) {
                IMSettingsRow(R.drawable.nola_sun, R.string.appearance_title) { detailSection = "appearance" }
                IMSettingsRow(R.drawable.nola_bell, R.string.notifications_title) { detailSection = "notifications" }
                IMSettingsRow(R.drawable.nola_review, R.string.settings_privacy) { detailSection = "privacy" }
            }
            Column(Modifier.fillMaxWidth().nolaSurface(radius = 14.dp)) {
                IMSettingsRow(R.drawable.nola_folder, R.string.settings_data_sources, onOpenDataSources)
                IMSettingsRow(R.drawable.nola_calendar, R.string.scheduled_title, onScheduledActions)
            }
            return@NolaPage
        }

        if (state.loadFailed) {
            NolaCard(plain = true) {
                Text(
                    stringResource(R.string.settings_load_failed),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.error
                )
                Text(
                    stringResource(R.string.settings_load_failed_detail),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Button(onClick = viewModel::refresh, enabled = !state.isLoading) {
                    Text(stringResource(R.string.common_retry))
                }
            }
        }

        operationMessage?.let { message ->
            SettingsFeedbackCard(
                message = message,
                isError = operationFailed,
                onDismiss = { operationMessage = null }
            )
        }

        if (detailSection == "notifications") {
            SettingsNotificationsCard(onNotifications, { openNotificationSettings(context) }, notificationsAllowed)
        }
        if (detailSection == "appearance") {
        NolaCard(plain = true) {
            NolaSectionTitle(title = stringResource(R.string.appearance_title))
            Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer, NolaContinuousShape(10.dp)).padding(3.dp).selectableGroup(), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                AppearancePreference.entries.forEach { choice ->
                    Box(Modifier.weight(1f).heightIn(min = 44.dp)
                        .background(if (appearance == choice) MaterialTheme.colorScheme.surface else androidx.compose.ui.graphics.Color.Transparent, NolaContinuousShape(8.dp))
                        .selectable(selected = appearance == choice, role = Role.RadioButton,
                            onClick = { viewModel.setAppearance(choice) }), contentAlignment = Alignment.Center) {
                        Text(stringResource(when (choice) {
                            AppearancePreference.System -> R.string.appearance_system
                            AppearancePreference.Light -> R.string.appearance_light
                            AppearancePreference.Dark -> R.string.appearance_dark
                        }), fontSize = 14.sp, maxLines = 1)
                    }
                }
            }
        }

        }

        if (detailSection == "privacy") {
        NolaCard(plain = true) {
            NolaSectionTitle(title = stringResource(R.string.settings_data_management))
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 56.dp)
                    .padding(vertical = NolaSpacing.sm),
                verticalAlignment = Alignment.CenterVertically
            ) {
                NolaIconBadge(icon = Icons.Filled.Storage, size = 28.dp)
                Spacer(Modifier.width(NolaSpacing.sm))
                Column(modifier = Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_local_data_size), style = MaterialTheme.typography.bodyMedium)
                    Text(sizeStr, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f))
            SettingsNavRow(
                label = stringResource(R.string.settings_export_data),
                description = if (isExportingData) stringResource(R.string.settings_exporting_data) else stringResource(R.string.settings_export_desc),
                icon = Icons.Filled.FileUpload,
                isBusy = isExportingData,
                enabled = !isExportingData && !isClearingData,
                onClick = { showExportDialog = true }
            )
            HorizontalDivider(
                modifier = Modifier.padding(vertical = NolaSpacing.xs),
                color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f)
            )
            NolaSectionTitle(title = stringResource(R.string.settings_privacy))
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 52.dp)
                    .padding(top = NolaSpacing.xs, bottom = NolaSpacing.sm),
                verticalAlignment = Alignment.CenterVertically
            ) {
                NolaIconBadge(icon = Icons.Filled.Shield, size = 28.dp)
                Spacer(Modifier.width(NolaSpacing.sm))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        stringResource(R.string.settings_privacy_desc),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        NolaCard(plain = true) {
            NolaSectionTitle(title = stringResource(R.string.settings_about))
            val versionName = remember(context) {
                runCatching {
                    context.packageManager.getPackageInfo(context.packageName, 0).versionName
                }.getOrNull().orEmpty().ifBlank { "—" }
            }
            SettingsInfoRow(stringResource(R.string.settings_version), versionName)
            SoftwareUpdateControls()
            SettingsCopyRow(
                label = stringResource(R.string.settings_client_uuid),
                value = state.clientUuid.ifBlank { stringResource(R.string.settings_generating) },
                enabled = state.clientUuid.isNotBlank()
            ) {
                if (state.clientUuid.isNotBlank()) {
                    val clipboard = context.getSystemService(ClipboardManager::class.java)
                    clipboard.setPrimaryClip(ClipData.newPlainText(context.getString(R.string.settings_client_uuid), state.clientUuid))
                    Toast.makeText(context, context.getString(R.string.settings_client_uuid_copied), Toast.LENGTH_SHORT).show()
                }
            }
        }

        NolaCard(plain = true) {
            NolaSectionTitle(title = stringResource(R.string.settings_reset))
            Text(
                stringResource(R.string.settings_reset_detail),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(bottom = NolaSpacing.xs)
            )
            SettingsActionRow(
                label = stringResource(R.string.settings_clear_all_data),
                icon = Icons.Filled.Delete,
                tint = NolaColors.danger,
                isBusy = isClearingData,
                enabled = !isExportingData && !isClearingData
            ) { showDeleteDialog = true }
        }
        }
    }
}

@Composable
private fun IMSettingsRow(icon: Int, label: Int, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(onClick = onClick).heightIn(min = 62.dp).padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(painterResource(icon), null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.primary)
        Text(stringResource(label), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Icon(painterResource(R.drawable.nola_chevron), null, Modifier.size(15.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SettingsConnectionsCard(
    onOpenDataSources: () -> Unit,
    onOpenModels: () -> Unit,
    onOpenIntelligence: () -> Unit
) {
    NolaCard(plain = true) {
        NolaSectionTitle(title = stringResource(R.string.settings_connections))
        SettingsNavRow(
            label = stringResource(R.string.settings_data_sources),
            description = stringResource(R.string.settings_data_sources_desc),
            icon = Icons.Filled.Dataset,
            onClick = onOpenDataSources
        )
        HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f))
        SettingsNavRow(
            label = stringResource(R.string.models_title),
            description = stringResource(R.string.settings_models_desc),
            icon = Icons.Filled.Memory,
            onClick = onOpenModels
        )
        HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f))
        SettingsNavRow(
            label = stringResource(R.string.settings_capabilities),
            description = stringResource(R.string.settings_capabilities_desc),
            icon = Icons.Filled.Extension,
            onClick = onOpenIntelligence
        )
    }
}

@Composable
private fun SettingsAutomationCard(
    onScheduledActions: () -> Unit
) {
    NolaCard(plain = true) {
        NolaSectionTitle(title = stringResource(R.string.settings_automation))
        SettingsNavRow(
            label = stringResource(R.string.scheduled_title),
            description = stringResource(R.string.settings_scheduled_actions_desc),
            icon = Icons.Filled.CalendarMonth,
            onClick = onScheduledActions
        )
    }
}

internal const val SETTINGS_NOTIFICATION_CENTER_TEST_TAG = "settings_notification_center"
internal const val SETTINGS_NOTIFICATION_PERMISSION_TEST_TAG = "settings_notification_permission"

@Composable
internal fun SettingsNotificationsCard(
    onNotifications: () -> Unit,
    onNotificationSettings: () -> Unit,
    notificationsAllowed: Boolean
) {
    NolaCard(plain = true) {
        NolaSectionTitle(title = stringResource(R.string.notifications_title))
        SettingsNavRow(
            label = stringResource(R.string.notifications_title),
            description = stringResource(R.string.settings_notification_center_desc),
            icon = Icons.Filled.Notifications,
            modifier = Modifier.testTag(SETTINGS_NOTIFICATION_CENTER_TEST_TAG),
            onClick = onNotifications
        )
        HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f))
        SettingsNavRow(
            label = stringResource(R.string.settings_notification_permissions),
            description = stringResource(
                if (notificationsAllowed) R.string.settings_notification_permissions_allowed
                else R.string.settings_notification_permissions_blocked
            ),
            icon = if (notificationsAllowed) Icons.Filled.Notifications else Icons.Filled.NotificationsOff,
            modifier = Modifier.testTag(SETTINGS_NOTIFICATION_PERMISSION_TEST_TAG),
            onClick = onNotificationSettings
        )
    }
}

private fun openNotificationSettings(context: Context) {
    val notificationIntent = Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
        putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
    }
    runCatching { context.startActivity(notificationIntent) }
        .onFailure {
            context.startActivity(
                Intent(
                    Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    Uri.parse("package:${context.packageName}")
                )
            )
        }
}

@Composable
private fun SettingsFeedbackCard(
    message: String,
    isError: Boolean,
    onDismiss: () -> Unit
) {
    val tint = if (isError) MaterialTheme.colorScheme.error else NolaColors.success
    NolaCard(plain = true) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                if (isError) Icons.Filled.ErrorOutline else Icons.Filled.CheckCircle,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(20.dp)
            )
            Spacer(Modifier.width(NolaSpacing.sm))
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                color = if (isError) tint else MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f)
            )
            IconButton(onClick = onDismiss) {
                Icon(
                    Icons.Filled.Close,
                    contentDescription = stringResource(R.string.common_dismiss),
                    modifier = Modifier.size(18.dp)
                )
            }
        }
    }
}

@Composable
fun SettingsNavRow(
    label: String,
    description: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    isBusy: Boolean = false,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
    onClick: () -> Unit
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, role = Role.Button) { onClick() }
            .heightIn(min = 56.dp)
            .padding(vertical = NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        NolaIconBadge(icon = icon, size = 32.dp)
        Spacer(Modifier.width(NolaSpacing.sm))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                label,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis
            )
        }
        Box(
            modifier = Modifier.size(24.dp),
            contentAlignment = Alignment.Center
        ) {
            if (isBusy) {
                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            } else {
                Icon(
                    Icons.AutoMirrored.Filled.KeyboardArrowRight, null,
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                )
            }
        }
    }
}

@Composable
fun SettingsInfoRow(label: String, value: String) {
    BoxWithConstraints(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .padding(vertical = NolaSpacing.sm)
    ) {
        if (maxWidth >= 360.dp) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                Text(
                    value,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.End,
                    modifier = Modifier.weight(1f)
                )
            }
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(label, style = MaterialTheme.typography.bodyMedium)
                Text(
                    value,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
fun SettingsCopyRow(label: String, value: String, enabled: Boolean = true, onClick: () -> Unit) {
    BoxWithConstraints(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, role = Role.Button) { onClick() }
            .heightIn(min = 56.dp)
            .padding(vertical = NolaSpacing.sm)
    ) {
        if (maxWidth >= 360.dp) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                Row(modifier = Modifier.weight(1f), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        value,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        textAlign = TextAlign.End,
                        modifier = Modifier.weight(1f, fill = false)
                    )
                    if (enabled) {
                        Spacer(Modifier.width(6.dp))
                        Icon(Icons.Filled.ContentCopy, contentDescription = null, modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.primary)
                    }
                }
            }
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(label, style = MaterialTheme.typography.bodyMedium)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        value,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f)
                    )
                    if (enabled) {
                        Spacer(Modifier.width(6.dp))
                        Icon(Icons.Filled.ContentCopy, contentDescription = null, modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.primary)
                    }
                }
            }
        }
    }
}

@Composable
fun SettingsActionRow(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    tint: androidx.compose.ui.graphics.Color = MaterialTheme.colorScheme.error,
    isBusy: Boolean = false,
    enabled: Boolean = true,
    onClick: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, role = Role.Button) { onClick() }
            .heightIn(min = 56.dp)
            .padding(vertical = NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Box(
            modifier = Modifier.size(24.dp),
            contentAlignment = Alignment.Center
        ) {
            if (isBusy) {
                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = tint)
            } else {
                Icon(icon, null, Modifier.size(18.dp), tint = tint)
            }
        }
        Spacer(Modifier.width(NolaSpacing.sm))
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = tint,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f)
        )
    }
}
