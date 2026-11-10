package com.nola.ui.screen.home

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.BackHandler
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.TrendingUp
import androidx.compose.material.icons.filled.CalendarToday
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaDivider
import com.nola.ui.components.NolaIconBadge
import com.nola.ui.components.NolaPage
import com.nola.ui.components.NolaPageHeader
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaStatCard
import com.nola.ui.screen.notifications.NotificationCenterScreen
import com.nola.ui.screen.notifications.NotificationCenterViewModel
import com.nola.ui.screen.scheduled.ScheduledActionsScreen
import com.nola.ui.screen.scheduled.ScheduledActionsUiState
import com.nola.ui.screen.scheduled.ScheduledActionsViewModel
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(onCreateInChat: () -> Unit = {}) {
    var activeScreen by rememberSaveable { mutableStateOf<String?>(null) }
    BackHandler(enabled = activeScreen != null) { activeScreen = null }

    when (activeScreen) {
        "notifications" -> NotificationCenterScreen(onBack = { activeScreen = null })
        "scheduled" -> ScheduledActionsScreen(onBack = { activeScreen = null }, onCreateInChat = onCreateInChat)
        else -> HomeDashboard(onNotifications = { activeScreen = "notifications" }, onScheduledActions = { activeScreen = "scheduled" })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeDashboard(
    onNotifications: () -> Unit = {},
    onScheduledActions: () -> Unit = {},
    scheduledActionsViewModel: ScheduledActionsViewModel = hiltViewModel(),
    notificationCenterViewModel: NotificationCenterViewModel = hiltViewModel(),
    dataSourcesViewModel: DataSourcesViewModel = hiltViewModel()
) {
    val scheduledState by scheduledActionsViewModel.uiState.collectAsStateWithLifecycle()
    val notificationState by notificationCenterViewModel.uiState.collectAsStateWithLifecycle()
    val dataSourcesState by dataSourcesViewModel.uiState.collectAsStateWithLifecycle()
    val healthSummary = dataSourcesState.health
    val needsHealthPermission = healthSummary?.availability == AndroidHealthAvailability.Available &&
        (healthSummary.permissionState == AndroidHealthPermissionState.Missing || healthSummary.missingPermissionCount > 0)
    val healthPermissionLauncher = rememberLauncherForActivityResult(
        AndroidHealthDataSource.permissionContract()
    ) {
        dataSourcesViewModel.refresh()
    }
    val now = System.currentTimeMillis()
    val dateStr = remember(now) {
        SimpleDateFormat("EEEE, MMMM d", Locale.getDefault()).format(Date(now))
    }

    LaunchedEffect(Unit) {
        dataSourcesViewModel.refresh()
    }
    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    androidx.compose.runtime.DisposableEffect(lifecycleOwner) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
                dataSourcesViewModel.refresh()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

    NolaPage {
        NolaPageHeader(
            title = stringResource(R.string.data_title),
            subtitle = stringResource(R.string.data_subtitle)
        ) {
            NotificationBellButton(
                unreadCount = notificationState.unreadCount,
                onClick = onNotifications
            )
        }

        DataOverviewStatusCard(dataState = dataSourcesState, scheduledState = scheduledState)

        Box(modifier = Modifier.testTag("data.overview.health")) {
        NolaCard {
            val healthActionText = when {
                dataSourcesState.isRefreshing -> null
                needsHealthPermission -> stringResource(R.string.data_health_connect)
                else -> stringResource(R.string.common_refresh)
            }
            NolaSectionTitle(
                icon = Icons.Filled.Favorite,
                title = stringResource(R.string.data_phone_health),
                action = {
                    if (dataSourcesState.isRefreshing) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp),
                            strokeWidth = 2.dp
                        )
                    } else if (healthActionText != null) {
                        Text(
                            healthActionText,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier
                                .clip(RoundedCornerShape(NolaRadius.pill))
                                .clickable {
                                    if (needsHealthPermission) {
                                        healthPermissionLauncher.launch(AndroidHealthDataSource.readPermissions)
                                    } else {
                                        dataSourcesViewModel.refresh()
                                    }
                                }
                                .heightIn(min = 48.dp)
                                .padding(horizontal = 14.dp, vertical = 8.dp)
                        )
                    }
                }
            )

            val healthMessage = when {
                healthSummary?.error?.isNotBlank() == true -> healthSummary.error
                healthSummary?.availability == AndroidHealthAvailability.Unavailable -> stringResource(R.string.data_health_unavailable)
                healthSummary?.availability == AndroidHealthAvailability.ProviderUpdateRequired -> stringResource(R.string.data_health_update_required)
                healthSummary?.permissionState == AndroidHealthPermissionState.Missing -> stringResource(R.string.data_health_permission_needed)
                healthSummary?.permissionState == AndroidHealthPermissionState.Granted && healthSummary.missingPermissionCount > 0 -> stringResource(R.string.data_health_partial_permission)
                healthSummary?.permissionState == AndroidHealthPermissionState.Granted && !healthSummary.hasHealthData -> stringResource(R.string.data_no_health_data)
                healthSummary?.permissionState == AndroidHealthPermissionState.Granted -> stringResource(R.string.data_health_local_summary)
                else -> stringResource(R.string.data_health_pending)
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(NolaRadius.lg))
                    .background(NolaColors.success.copy(alpha = 0.08f))
                    .padding(NolaSpacing.md),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        stringResource(R.string.data_steps),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Text(
                        healthSummary?.totalSteps
                            ?.takeIf { it > 0 }
                            ?.let { java.text.NumberFormat.getIntegerInstance(Locale.getDefault()).format(it) }
                            ?: "—",
                        style = MaterialTheme.typography.headlineLarge,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface
                    )
                    healthSummary?.activityDate?.let { activityDate ->
                        Text(
                            java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM, Locale.getDefault())
                                .format(Date.from(activityDate)),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
                NolaIconBadge(
                    icon = Icons.AutoMirrored.Filled.TrendingUp,
                    size = 52.dp,
                    tint = NolaColors.success
                )
            }

            val sleep = healthSummary?.averageSleepHoursPerDay
            val workoutType = healthSummary?.latestWorkoutType
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = NolaSpacing.sm),
                horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
            ) {
                NolaStatCard(
                    title = stringResource(R.string.data_heart_rate),
                    value = healthSummary?.averageHeartRateBpm?.takeIf { it > 0 }?.let { "$it bpm" } ?: "—",
                    icon = Icons.Filled.Favorite,
                    modifier = Modifier.weight(1f),
                    tint = NolaColors.danger
                )
                NolaStatCard(
                    title = stringResource(R.string.data_sleep),
                    value = sleep?.takeIf { it > 0.0 }?.let { stringResource(R.string.data_sleep_hours_format, it) } ?: "—",
                    icon = Icons.Filled.Schedule,
                    modifier = Modifier.weight(1f),
                    tint = MaterialTheme.colorScheme.primary
                )
            }

            if (workoutType != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = NolaSpacing.sm)
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.55f))
                        .padding(horizontal = NolaSpacing.sm, vertical = NolaSpacing.xs),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    NolaIconBadge(
                        icon = Icons.AutoMirrored.Filled.TrendingUp,
                        size = 32.dp,
                        tint = NolaColors.warning
                    )
                    Spacer(Modifier.width(NolaSpacing.sm))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            stringResource(R.string.data_workout),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            stringResource(
                                R.string.data_workout_format,
                                workoutType,
                                healthSummary.latestWorkoutMinutes ?: 0,
                                healthSummary.latestWorkoutDaysAgo ?: 0
                            ),
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = FontWeight.Medium
                        )
                    }
                }
            }

            Row(
                modifier = Modifier.padding(top = NolaSpacing.sm),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Box(
                    modifier = Modifier
                        .size(7.dp)
                        .clip(CircleShape)
                        .background(
                            if (healthSummary?.hasHealthData == true) NolaColors.success
                            else MaterialTheme.colorScheme.outline
                        )
                )
                Spacer(Modifier.width(NolaSpacing.xs))
                Text(
                    healthMessage,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f)
                )
                healthSummary?.updatedAt?.let { updatedAt ->
                    Text(
                        java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT, Locale.getDefault())
                            .format(Date.from(updatedAt)),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.outline
                    )
                }
            }
        }
        }

        ScheduledOverviewCard(state = scheduledState, onClick = onScheduledActions)

        TodayOverviewCard(dateText = dateStr)

    }
}

@Composable
fun DataOverviewStatusCard(
    dataState: DataSourcesUiState,
    scheduledState: ScheduledActionsUiState
) {
    val health = dataState.health
    val sourceCount = dataState.customSources.count { it.enabled } +
        if (health?.permissionState == AndroidHealthPermissionState.Granted) 1 else 0
    val healthStatus = when {
        dataState.isRefreshing -> stringResource(R.string.data_sources_status_updating)
        health?.permissionState == AndroidHealthPermissionState.Granted && health.missingPermissionCount == 0 ->
            stringResource(R.string.data_sources_status_ready)
        health?.permissionState == AndroidHealthPermissionState.Granted ->
            stringResource(R.string.data_sources_status_partial)
        health?.availability == AndroidHealthAvailability.Unavailable ->
            stringResource(R.string.data_sources_status_unavailable)
        else -> stringResource(R.string.data_sources_status_access_needed)
    }
    val activeCount = scheduledState.actions.count { it.enabled }

    NolaCard {
        BoxWithConstraints(modifier = Modifier.fillMaxWidth().testTag("data.overview.status")) {
            if (maxWidth >= 560.dp) {
                Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    DataOverviewMetric(stringResource(R.string.data_workspace_sources), sourceCount.toString(), MaterialTheme.colorScheme.primary, Modifier.weight(1f))
                    DataOverviewVerticalDivider()
                    DataOverviewMetric(
                        stringResource(R.string.data_phone_health),
                        healthStatus,
                        if (health?.permissionState == AndroidHealthPermissionState.Granted) NolaColors.success else NolaColors.warning,
                        Modifier.weight(1f)
                    )
                    DataOverviewVerticalDivider()
                    DataOverviewMetric(stringResource(R.string.data_scheduled_actions), activeCount.toString(), NolaColors.warning, Modifier.weight(1f))
                }
            } else {
                Column {
                    DataOverviewStatusRow(stringResource(R.string.data_workspace_sources), sourceCount.toString(), MaterialTheme.colorScheme.primary)
                    NolaDivider()
                    DataOverviewStatusRow(
                        stringResource(R.string.data_phone_health),
                        healthStatus,
                        if (health?.permissionState == AndroidHealthPermissionState.Granted) NolaColors.success else NolaColors.warning
                    )
                    NolaDivider()
                    DataOverviewStatusRow(stringResource(R.string.data_scheduled_actions), activeCount.toString(), NolaColors.warning)
                }
            }
        }
    }
}

@Composable
private fun DataOverviewMetric(label: String, value: String, tint: Color, modifier: Modifier = Modifier) {
    Column(modifier = modifier.padding(horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            Box(Modifier.size(7.dp).clip(CircleShape).background(tint))
            Text(value, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun DataOverviewVerticalDivider() {
    Box(Modifier.width(1.dp).heightIn(min = 38.dp).background(MaterialTheme.colorScheme.outlineVariant))
}

@Composable
private fun DataOverviewStatusRow(label: String, value: String, tint: Color) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(tint))
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold, color = tint)
    }
}

@Composable
fun ScheduledOverviewCard(state: ScheduledActionsUiState, onClick: () -> Unit) {
    val activeCount = state.actions.count { it.enabled }
    val nextAction = state.actions.firstOrNull { it.enabled }
    val latestResult = state.recentResults.maxByOrNull { it.createdAt }
    val latestDetail = latestResult?.failureReason ?: latestResult?.llmSummaryText

    Box(modifier = Modifier.testTag("data.overview.automations")) {
        NolaCard {
            NolaSectionTitle(
                icon = Icons.Filled.Schedule,
                title = stringResource(R.string.data_scheduled_actions),
                action = {
                    Text(
                        stringResource(R.string.data_all),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier
                            .clip(RoundedCornerShape(NolaRadius.pill))
                            .clickable(onClick = onClick)
                            .testTag("data.overview.automations.open")
                            .heightIn(min = 48.dp)
                            .padding(horizontal = 14.dp, vertical = 8.dp)
                    )
                }
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                NolaIconBadge(icon = Icons.Filled.Schedule, size = 40.dp, tint = NolaColors.warning)
                Spacer(Modifier.width(NolaSpacing.sm))
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Text(
                        if (state.actions.isEmpty()) stringResource(R.string.data_no_scheduled_actions)
                        else stringResource(R.string.data_active_count_format, activeCount, state.actions.size),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        nextAction?.name ?: stringResource(R.string.data_schedule_hint),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
            }
            if (latestResult != null) {
                Surface(
                    modifier = Modifier.fillMaxWidth().padding(top = NolaSpacing.sm),
                    shape = RoundedCornerShape(NolaRadius.md),
                    color = scheduledResultTint(latestResult.status).copy(alpha = 0.08f)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.Top,
                        horizontalArrangement = Arrangement.spacedBy(9.dp)
                    ) {
                        Box(Modifier.padding(top = 5.dp).size(7.dp).clip(CircleShape).background(scheduledResultTint(latestResult.status)))
                        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(scheduledResultLabel(latestResult.status), style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
                            if (!latestDetail.isNullOrBlank()) {
                                Text(
                                    latestDetail,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun scheduledResultTint(status: String): Color = when (status.lowercase(Locale.ROOT)) {
    "success" -> NolaColors.success
    "partial" -> NolaColors.warning
    "pending", "running" -> MaterialTheme.colorScheme.primary
    else -> NolaColors.danger
}

@Composable
private fun scheduledResultLabel(status: String): String = when (status.lowercase(Locale.ROOT)) {
    "success" -> stringResource(R.string.scheduled_result_success)
    "partial" -> stringResource(R.string.scheduled_result_partial)
    "skipped" -> stringResource(R.string.scheduled_result_skipped)
    "pending", "running" -> stringResource(R.string.scheduled_result_running)
    else -> stringResource(R.string.scheduled_result_failed)
}

@Composable
fun TodayOverviewCard(dateText: String) {
    Box(modifier = Modifier.testTag("data.overview.today")) {
        NolaCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        Calendar.getInstance().get(Calendar.DAY_OF_MONTH).toString(),
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
                Spacer(Modifier.width(NolaSpacing.sm))
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(stringResource(R.string.data_today), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                    Text(dateText, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(stringResource(R.string.data_no_upcoming_events), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
                }
                Icon(Icons.Filled.CalendarToday, contentDescription = null, tint = MaterialTheme.colorScheme.outline)
            }
        }
    }
}

@Composable
private fun NotificationBellButton(
    unreadCount: Int,
    onClick: () -> Unit
) {
    Box(
        modifier = Modifier
            .size(48.dp)
            .clickable(role = androidx.compose.ui.semantics.Role.Button) { onClick() },
        contentAlignment = Alignment.Center
    ) {
        Box(
            modifier = Modifier
                .size(36.dp)
                .clip(RoundedCornerShape(NolaRadius.md))
                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.12f)),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                Icons.Filled.Notifications,
                contentDescription = stringResource(R.string.data_notifications),
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.primary
            )
        }
        if (unreadCount > 0) {
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .size(18.dp)
                    .clip(CircleShape)
                    .background(Color.Red)
                    .border(1.5.dp, MaterialTheme.colorScheme.surface, CircleShape),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = if (unreadCount > 9) "9+" else unreadCount.toString(),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = Color.White
                )
            }
        }
    }
}
