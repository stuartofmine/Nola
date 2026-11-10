package com.nola.ui.screen.notifications

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Circle
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaIconBadge
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import java.text.DateFormat
import java.util.Date
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotificationDetailScreen(
    notification: NotificationEntity,
    onBack: () -> Unit
) {
    val dateFmt = remember {
        DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT, Locale.getDefault())
    }
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    var showsTechnicalDetails by rememberSaveable { mutableStateOf(false) }
    val hasTechnicalDetails = !notification.dedupeKey.isNullOrBlank()

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            NolaTopAppBar(title = stringResource(R.string.notifications_detail_title), onBack = onBack)
        }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
            ,
            contentAlignment = Alignment.TopCenter
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 760.dp)
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.md)
            ) {
                NolaCard {
                    BoxWithConstraints {
                        if (maxWidth < 430.dp) {
                            Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)) {
                                NotificationIdentity(notification)
                                NotificationReadBadge(notification.isRead)
                            }
                        } else {
                            Row(verticalAlignment = Alignment.Top) {
                                Box(modifier = Modifier.weight(1f)) {
                                    NotificationIdentity(notification)
                                }
                                Spacer(Modifier.width(NolaSpacing.sm))
                                NotificationReadBadge(notification.isRead)
                            }
                        }
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))

                if (!notification.body.isNullOrEmpty()) {
                    NolaCard {
                        Text(
                            stringResource(R.string.notifications_message),
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(Modifier.height(NolaSpacing.sm))
                        SelectionContainer {
                            Text(
                                notification.body,
                                style = MaterialTheme.typography.bodyLarge
                            )
                        }
                    }
                    Spacer(Modifier.height(NolaSpacing.md))
                }

                NolaCard {
                    Text(stringResource(R.string.notifications_timeline), style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                    Spacer(Modifier.height(NolaSpacing.sm))
                    TimelineRow(
                        label = stringResource(R.string.notifications_created),
                        time = dateFmt.format(Date(notification.createdAt)),
                        icon = Icons.Filled.Notifications
                    )
                    notification.readAt?.let { readAt ->
                        Spacer(Modifier.height(NolaSpacing.xs))
                        TimelineRow(
                            label = stringResource(R.string.notifications_read),
                            time = dateFmt.format(Date(readAt)),
                            icon = Icons.Filled.Circle,
                            tint = NolaColors.success
                        )
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))

                NolaCard {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            stringResource(R.string.notifications_details),
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.weight(1f)
                        )
                        if (hasTechnicalDetails) {
                            IconButton(onClick = { showsTechnicalDetails = !showsTechnicalDetails }) {
                                Icon(
                                    if (showsTechnicalDetails) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                                    stringResource(
                                        if (showsTechnicalDetails) R.string.notifications_hide_technical
                                        else R.string.notifications_show_technical
                                    )
                                )
                            }
                        }
                    }
                    InfoRow(stringResource(R.string.notifications_source), notificationDetailSourceLabel(notification.source))
                    InfoRow(stringResource(R.string.notifications_type), notificationTypeLabel(notification.notificationType))
                    if (showsTechnicalDetails && hasTechnicalDetails) {
                        notification.dedupeKey?.takeIf { it.isNotBlank() }?.let { key ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                InfoRow(
                                    stringResource(R.string.notifications_dedupe_key),
                                    key,
                                    modifier = Modifier.weight(1f)
                                )
                                IconButton(onClick = {
                                    clipboard.setText(AnnotatedString(key))
                                    Toast.makeText(
                                        context,
                                        context.getString(R.string.notifications_identifier_copied),
                                        Toast.LENGTH_SHORT
                                    ).show()
                                }) {
                                    Icon(Icons.Filled.ContentCopy, stringResource(R.string.notifications_copy_identifier))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun TimelineRow(
    label: String,
    time: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    tint: Color = MaterialTheme.colorScheme.onSurfaceVariant
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(16.dp), tint = tint)
        Spacer(Modifier.width(NolaSpacing.sm))
        Column {
            Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(time, style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
fun InfoRow(label: String, value: String, modifier: Modifier = Modifier) {
    BoxWithConstraints(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = 5.dp)
    ) {
        if (maxWidth < 360.dp) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                SelectionContainer {
                    Text(value, style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold)
                }
            }
        } else {
            Row(horizontalArrangement = Arrangement.SpaceBetween) {
                Text(
                    label,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(0.42f)
                )
                SelectionContainer(modifier = Modifier.weight(0.58f)) {
                    Text(
                        value,
                        style = MaterialTheme.typography.bodySmall,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
        }
    }
}

@Composable
private fun NotificationIdentity(notification: NotificationEntity) {
    val sourceTint = notificationSourceTint(notification.source)
    Row(verticalAlignment = Alignment.Top) {
        NolaIconBadge(
            icon = notificationSourceIcon(notification.source),
            size = 40.dp,
            tint = sourceTint
        )
        Spacer(Modifier.width(NolaSpacing.md))
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            Text(
                notification.title,
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold
            )
            Text(
                notificationDetailSourceLabel(notification.source),
                style = MaterialTheme.typography.labelMedium,
                color = sourceTint,
                fontWeight = FontWeight.Medium
            )
        }
    }
}

@Composable
private fun NotificationReadBadge(isRead: Boolean) {
    Text(
        stringResource(if (isRead) R.string.notifications_read else R.string.notifications_unread),
        style = MaterialTheme.typography.labelMedium,
        color = if (isRead) MaterialTheme.colorScheme.onSurfaceVariant else NolaColors.primary,
        modifier = Modifier
            .clip(CircleShape)
            .background(
                if (isRead) MaterialTheme.colorScheme.surfaceVariant
                else NolaColors.primary.copy(alpha = 0.12f)
            )
            .padding(horizontal = 10.dp, vertical = 5.dp)
    )
}

@Composable
private fun notificationDetailSourceLabel(source: String): String = when (source.lowercase(Locale.ROOT)) {
    "calendar" -> stringResource(R.string.notifications_source_calendar)
    "reminder" -> stringResource(R.string.notifications_source_reminder)
    "url", "web" -> stringResource(R.string.notifications_source_web)
    "health" -> stringResource(R.string.notifications_source_health)
    "contact" -> stringResource(R.string.notifications_source_contact)
    else -> source
}

@Composable
private fun notificationTypeLabel(type: String): String = when (type.lowercase(Locale.ROOT)) {
    "message" -> stringResource(R.string.notifications_type_message)
    "reminder" -> stringResource(R.string.notifications_type_reminder)
    "task" -> stringResource(R.string.notifications_type_task)
    "automation" -> stringResource(R.string.notifications_type_automation)
    else -> type
}
