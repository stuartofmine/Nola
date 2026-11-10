package com.nola.ui.screen.scheduled

import android.text.format.DateFormat
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ScheduleTimeFields(
    frequency: String,
    value: String,
    zone: String,
    enabled: Boolean = true,
    onChange: (String, String) -> Unit,
) {
    var picker by rememberSaveable { mutableStateOf<String?>(null) }
    val zoneId = runCatching { ZoneId.of(zone) }.getOrNull()
    val local = scheduleLocalDateTime(value, zone)
    val time = scheduleLocalTime(value, zone)
    val date = local?.toLocalDate()
    val validation = validateScheduleTime(frequency, value, zone)
    val fallback = remember(zone) { LocalDateTime.now(zoneId ?: ZoneId.systemDefault()).withSecond(0).withNano(0) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (frequency == "once") {
            OutlinedButton(onClick = { picker = "date" }, enabled = enabled, modifier = Modifier.fillMaxWidth().testTag("schedule-date")) {
                Text(stringResource(R.string.scheduled_pick_date, date?.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM))
                    ?: stringResource(R.string.scheduled_not_selected)))
            }
        }
        OutlinedButton(onClick = { picker = "time" }, enabled = enabled, modifier = Modifier.fillMaxWidth().testTag("schedule-time")) {
            Text(stringResource(R.string.scheduled_pick_time, time?.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
                ?: stringResource(R.string.scheduled_not_selected)))
        }
        OutlinedButton(onClick = { picker = "zone" }, enabled = enabled, modifier = Modifier.fillMaxWidth().testTag("schedule-zone")) {
            Text(stringResource(R.string.scheduled_pick_zone, zone))
        }
        Text(stringResource(R.string.scheduled_zone_wall_time), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (frequency != "once") {
            Text(stringResource(R.string.scheduled_recurring_dst), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        when {
            zoneId == null -> Text(stringResource(R.string.scheduled_timezone_invalid), color = MaterialTheme.colorScheme.error)
            validation is ScheduleTimeValidation.Gap -> Text(stringResource(R.string.scheduled_dst_gap),
                color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("schedule-gap"))
            validation is ScheduleTimeValidation.Overlap -> {
                Text(stringResource(R.string.scheduled_dst_overlap), modifier = Modifier.testTag("schedule-overlap"))
                validation.offsets.forEachIndexed { index, offset ->
                    OutlinedButton(onClick = {
                        local?.let { onChange(it.atOffset(offset).toString(), zone) }
                    }, enabled = enabled, modifier = Modifier.fillMaxWidth().testTag("schedule-overlap-$index")) {
                        Text(stringResource(if (index == 0) R.string.scheduled_dst_first else R.string.scheduled_dst_second,
                            "UTC${if (offset == ZoneOffset.UTC) "+00:00" else offset.id}"))
                    }
                }
            }
            validation is ScheduleTimeValidation.Valid && frequency == "once" -> {
                // Keeps the selected occurrence legible after resolving an overlap.
                Text(stringResource(R.string.scheduled_exact_time, validation.wireValue),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            value.isNotBlank() && validation is ScheduleTimeValidation.Invalid -> Text(
                stringResource(if (frequency == "once") R.string.scheduled_time_invalid_once else R.string.scheduled_time_invalid_recurring),
                color = MaterialTheme.colorScheme.error)
        }
    }

    if (picker == "date") {
        val initialDate = date ?: fallback.toLocalDate()
        val state = rememberDatePickerState(initialSelectedDateMillis = initialDate
            .atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli(),
            yearRange = minOf(1900, initialDate.year)..maxOf(2100, initialDate.year))
        DatePickerDialog(onDismissRequest = { picker = null }, confirmButton = {
            TextButton(onClick = {
                state.selectedDateMillis?.let {
                    val selected = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()
                    if (selected != date) onChange(LocalDateTime.of(selected, time ?: fallback.toLocalTime()).toString(), zone)
                }
                picker = null
            }, enabled = state.selectedDateMillis != null, modifier = Modifier.testTag("schedule-picker-confirm")) {
                Text(stringResource(R.string.common_confirm))
            }
        }, dismissButton = {
            TextButton(onClick = { picker = null }, modifier = Modifier.testTag("schedule-picker-cancel")) { Text(stringResource(R.string.common_cancel)) }
        }) { DatePicker(state) }
    }
    if (picker == "time") {
        val state = rememberTimePickerState(initialHour = (time ?: fallback.toLocalTime()).hour,
            initialMinute = (time ?: fallback.toLocalTime()).minute,
            is24Hour = DateFormat.is24HourFormat(LocalContext.current))
        AlertDialog(onDismissRequest = { picker = null }, title = { Text(stringResource(R.string.scheduled_time)) },
            text = {
                if (LocalConfiguration.current.screenHeightDp < 600 || LocalConfiguration.current.fontScale > 1.3f) TimeInput(state)
                else TimePicker(state)
            }, confirmButton = {
                TextButton(onClick = {
                    val selected = LocalTime.of(state.hour, state.minute)
                    if (selected != time) onChange(if (frequency == "once") LocalDateTime.of(date ?: fallback.toLocalDate(), selected).toString()
                        else selected.format(DateTimeFormatter.ofPattern("HH:mm")), zone)
                    picker = null
                }, modifier = Modifier.testTag("schedule-picker-confirm")) { Text(stringResource(R.string.common_confirm)) }
            }, dismissButton = {
                TextButton(onClick = { picker = null }, modifier = Modifier.testTag("schedule-picker-cancel")) { Text(stringResource(R.string.common_cancel)) }
            })
    }
    if (picker == "zone") {
        ScheduleZoneDialog(zone, onDismiss = { picker = null }, onConfirm = { selected ->
            if (selected != zone) onChange(scheduleValueForZoneChange(frequency, value, zone), selected)
            picker = null
        })
    }
}

@Composable
private fun ScheduleZoneDialog(zone: String, onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var selected by rememberSaveable { mutableStateOf(zone) }
    var query by rememberSaveable { mutableStateOf("") }
    val zones = remember { (ZoneId.getAvailableZoneIds() + "UTC").sorted() }
    val visible = remember(query) { zones.filter { it.contains(query.trim(), ignoreCase = true) } }
    AlertDialog(onDismissRequest = onDismiss, title = { Text(stringResource(R.string.scheduled_timezone)) }, text = {
        Column {
            OutlinedTextField(query, onValueChange = { query = it }, singleLine = true,
                label = { Text(stringResource(R.string.scheduled_zone_search)) }, modifier = Modifier.testTag("schedule-zone-search"))
            TextButton(onClick = { selected = ZoneId.systemDefault().id }) { Text(stringResource(R.string.scheduled_zone_device, ZoneId.systemDefault().id)) }
            Text(stringResource(R.string.scheduled_zone_selected, selected), style = MaterialTheme.typography.bodySmall)
            LazyColumn(Modifier.heightIn(max = 280.dp)) {
                if (visible.isEmpty()) item { Text(stringResource(R.string.scheduled_zone_no_matches)) }
                items(visible, key = { it }) { id ->
                    FilterChip(selected = selected == id, onClick = { selected = id }, label = { Text(id) }, modifier = Modifier.fillMaxWidth())
                }
            }
        }
    }, confirmButton = {
        TextButton(onClick = { onConfirm(selected) }, enabled = runCatching { ZoneId.of(selected) }.isSuccess,
            modifier = Modifier.testTag("schedule-picker-confirm")) { Text(stringResource(R.string.common_confirm)) }
    }, dismissButton = {
        TextButton(onClick = onDismiss, modifier = Modifier.testTag("schedule-picker-cancel")) { Text(stringResource(R.string.common_cancel)) }
    })
}
