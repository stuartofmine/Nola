package com.nola.ui.screen.notifications

import android.text.format.DateUtils
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Event
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotificationCenterScreen(
    onBack: (() -> Unit)? = null,
    viewModel: NotificationCenterViewModel = hiltViewModel()
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    var selectedNotificationId by rememberSaveable { mutableStateOf<String?>(null) }
    var query by rememberSaveable { mutableStateOf("") }
    var unreadOnly by rememberSaveable { mutableStateOf(false) }
    val normalizedQuery = query.trim()
    val visibleNotifications = state.notifications.filter { notification ->
        (!unreadOnly || !notification.isRead) && (
            normalizedQuery.isEmpty()
                || notification.title.contains(normalizedQuery, ignoreCase = true)
                || notification.body.orEmpty().contains(normalizedQuery, ignoreCase = true)
                || notification.source.contains(normalizedQuery, ignoreCase = true)
        )
    }
    val selectedNotification = selectedNotificationId?.let { id -> state.notifications.firstOrNull { it.id == id } }

    BackHandler(enabled = selectedNotificationId != null) {
        selectedNotificationId = null
    }

    if (selectedNotificationId != null && selectedNotification != null) {
        NotificationDetailScreen(
            notification = selectedNotification,
            onBack = { selectedNotificationId = null }
        )
        return
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            NolaTopAppBar(
                title = stringResource(R.string.notifications_title),
                onBack = onBack
            ) {
                IconButton(onClick = { viewModel.loadNotifications() }, enabled = !state.isLoading) {
                    if (state.isLoading) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    } else {
                        Icon(Icons.Filled.Refresh, stringResource(R.string.common_refresh))
                    }
                }
                if (state.unreadCount > 0) {
                    IconButton(onClick = viewModel::markAllRead, enabled = !state.isMarkingAllRead) {
                        if (state.isMarkingAllRead) {
                            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        } else {
                            Icon(Icons.Filled.DoneAll, stringResource(R.string.notifications_mark_all_read))
                        }
                    }
                }
            }
        }
    ) { padding ->
        Box(
            modifier = Modifier.fillMaxSize().padding(padding),
            contentAlignment = Alignment.TopCenter
        ) {
        Column(
            modifier = Modifier
                .widthIn(max = 760.dp)
                .fillMaxSize()
        ) {
            if (state.notifications.isNotEmpty()) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.sm),
                    singleLine = true,
                    leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
                    trailingIcon = {
                        AnimatedVisibility(
                            visible = query.isNotEmpty(),
                            enter = fadeIn(tween(NolaMotion.fast)),
                            exit = fadeOut(tween(NolaMotion.fast))
                        ) {
                            IconButton(onClick = { query = "" }) {
                                Icon(Icons.Filled.Close, stringResource(R.string.notifications_clear_search))
                            }
                        }
                    },
                    placeholder = { Text(stringResource(R.string.notifications_search_hint)) }
                )

                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = NolaSpacing.lg),
                    horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
                ) {
                    FilterChip(
                        selected = !unreadOnly,
                        onClick = { unreadOnly = false },
                        label = {
                            Text("${stringResource(R.string.notifications_filter_all_v2)} (${state.notifications.size})")
                        }
                    )
                    FilterChip(
                        selected = unreadOnly,
                        onClick = { unreadOnly = true },
                        label = {
                            Text("${stringResource(R.string.notifications_filter_unread_v2)} (${state.unreadCount})")
                        }
                    )
                }
            }

            AnimatedVisibility(
                visible = (state.hasSyncWarning || state.hasLoadError) && state.notifications.isNotEmpty(),
                enter = fadeIn(tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing)) +
                    expandVertically(tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing)),
                exit = fadeOut(tween(NolaMotion.fast)) +
                    shrinkVertically(tween(NolaMotion.fast))
            ) {
                Surface(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.sm),
                    color = MaterialTheme.colorScheme.tertiaryContainer.copy(alpha = 0.55f),
                    shape = MaterialTheme.shapes.medium
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        Icon(Icons.Filled.ErrorOutline, contentDescription = null, tint = MaterialTheme.colorScheme.onTertiaryContainer)
                        Text(
                            stringResource(
                                if (state.hasLoadError) R.string.notifications_load_failed_message
                                else R.string.notifications_sync_warning
                            ),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onTertiaryContainer,
                            modifier = Modifier.weight(1f)
                        )
                        if (state.hasLoadError) {
                            Button(onClick = viewModel::loadNotifications, enabled = !state.isLoading) {
                                Text(stringResource(R.string.notifications_retry))
                            }
                        }
                    }
                }
            }

            when {
                state.isLoading && state.notifications.isEmpty() -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
                state.hasLoadError && state.notifications.isEmpty() -> Column(
                    modifier = Modifier.fillMaxWidth().padding(NolaSpacing.xl),
                    horizontalAlignment = Alignment.Start,
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    NolaEmptyState(
                        icon = Icons.Filled.ErrorOutline,
                        title = stringResource(R.string.notifications_load_failed),
                        message = stringResource(R.string.notifications_load_failed_message)
                    )
                    Button(onClick = viewModel::loadNotifications) { Text(stringResource(R.string.notifications_retry)) }
                }
                visibleNotifications.isEmpty() -> Column(
                    modifier = Modifier.fillMaxWidth().padding(NolaSpacing.xl),
                    horizontalAlignment = Alignment.Start,
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Text(stringResource(when {
                        normalizedQuery.isNotEmpty() -> R.string.notifications_empty_search_title
                        unreadOnly -> R.string.notifications_empty_unread_title
                        else -> R.string.notifications_no_notifications
                    }), style = MaterialTheme.typography.titleMedium)
                    Text(stringResource(when {
                        normalizedQuery.isNotEmpty() -> R.string.notifications_empty_search_message
                        unreadOnly -> R.string.notifications_empty_unread_message
                        else -> R.string.notifications_empty_message
                    }), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (normalizedQuery.isNotEmpty() || unreadOnly) {
                        Button(onClick = {
                            query = ""
                            unreadOnly = false
                        }) {
                            Text(stringResource(R.string.notifications_reset_filters))
                        }
                    }
                }
                else -> LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(NolaSpacing.lg),
                    verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
                ) {
                    items(visibleNotifications, key = { it.id }) { notification ->
                        NotificationCard(
                            title = notification.title,
                            body = notification.body,
                            sourceKey = notification.source,
                            source = notificationSourceLabel(notification.source),
                            isRead = notification.isRead,
                            createdAt = notification.createdAt,
                            onClick = {
                                selectedNotificationId = notification.id
                                viewModel.markRead(notification.id)
                            }
                        )
                    }
                }
            }
        }
        }
    }
}

@Composable
fun NotificationCard(
    title: String,
    body: String?,
    sourceKey: String,
    source: String,
    isRead: Boolean,
    createdAt: Long,
    onClick: () -> Unit
) {
    val unreadLabel = stringResource(R.string.notifications_filter_unread_v2)
    val relativeTime = remember(createdAt) {
        DateUtils.getRelativeTimeSpanString(
            createdAt,
            System.currentTimeMillis(),
            DateUtils.MINUTE_IN_MILLIS,
            DateUtils.FORMAT_ABBREV_RELATIVE
        ).toString()
    }
    val sourceTint = notificationSourceTint(sourceKey)
    val containerColor by animateColorAsState(
        targetValue = if (isRead) {
            MaterialTheme.colorScheme.surface
        } else {
            MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.22f)
        },
        animationSpec = tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing),
        label = "notification-read-state"
    )

    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onClick),
        color = containerColor,
        tonalElevation = if (isRead) 1.dp else 2.dp,
        shape = MaterialTheme.shapes.large
    ) {
        Row(
            modifier = Modifier.padding(horizontal = NolaSpacing.md, vertical = 14.dp),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(NolaSpacing.md)
        ) {
            Box(
                modifier = Modifier
                    .size(38.dp)
                    .background(sourceTint.copy(alpha = 0.12f), CircleShape),
                contentAlignment = Alignment.Center
            ) {
                Icon(
                    imageVector = notificationSourceIcon(sourceKey),
                    contentDescription = null,
                    tint = sourceTint,
                    modifier = Modifier.size(19.dp)
                )
            }

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        title,
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.weight(1f),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis
                    )
                    AnimatedVisibility(
                        visible = !isRead,
                        enter = fadeIn(tween(NolaMotion.base)) + scaleIn(
                            initialScale = 0.90f,
                            animationSpec = tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing)
                        ),
                        exit = fadeOut(tween(NolaMotion.fast)) + scaleOut(
                            targetScale = 0.90f,
                            animationSpec = tween(NolaMotion.fast)
                        )
                    ) {
                        Row {
                            Spacer(Modifier.width(8.dp))
                            Box(
                                modifier = Modifier
                                    .size(7.dp)
                                    .background(MaterialTheme.colorScheme.primary, CircleShape)
                                    .semantics { contentDescription = unreadLabel }
                            )
                        }
                    }
                }
                if (!body.isNullOrBlank()) {
                    Text(
                        body,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        source,
                        style = MaterialTheme.typography.labelMedium,
                        color = sourceTint,
                        fontWeight = FontWeight.Medium
                    )
                    Text(
                        relativeTime,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.76f)
                    )
                }
            }
        }
    }
}

internal fun notificationSourceIcon(source: String): ImageVector = when (source.lowercase(Locale.ROOT)) {
    "calendar" -> Icons.Filled.Event
    "reminder" -> Icons.Filled.CheckCircle
    "url", "web" -> Icons.Filled.Language
    "health" -> Icons.Filled.Favorite
    "contact" -> Icons.Filled.Person
    else -> Icons.Filled.Notifications
}

@Composable
internal fun notificationSourceTint(source: String): Color = when (source.lowercase(Locale.ROOT)) {
    "calendar" -> MaterialTheme.colorScheme.tertiary
    "reminder" -> MaterialTheme.colorScheme.primary
    "url", "web" -> MaterialTheme.colorScheme.secondary
    "health" -> MaterialTheme.colorScheme.error
    "contact" -> MaterialTheme.colorScheme.primary
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
private fun notificationSourceLabel(source: String): String = when (source.lowercase(Locale.ROOT)) {
    "calendar" -> stringResource(R.string.notifications_source_calendar)
    "reminder" -> stringResource(R.string.notifications_source_reminder)
    "url", "web" -> stringResource(R.string.notifications_source_web)
    "health" -> stringResource(R.string.notifications_source_health)
    "contact" -> stringResource(R.string.notifications_source_contact)
    else -> source
}
