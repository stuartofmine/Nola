package com.nola.ui.generative

import androidx.compose.ui.res.stringResource
import com.nola.R

import android.app.Dialog
import android.app.DatePickerDialog
import android.app.TimePickerDialog
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.time.LocalDate
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter

/** Native editing only; Core validates the submitted range and wire value. */
@Composable
internal fun GenerativeUiDateInput(
    label: String, value: String, enableDate: Boolean, enableTime: Boolean,
    enabled: Boolean, changed: (String) -> Unit,
) {
    val context = LocalContext.current
    val latestChanged by rememberUpdatedState(changed)
    val latestEnabled by rememberUpdatedState(enabled)
    val lifetime = remember(context, enableDate, enableTime) { PickerLifetime() }
    DisposableEffect(lifetime) {
        lifetime.active = true
        onDispose {
            lifetime.active = false
            lifetime.dialog?.dismiss()
            lifetime.dialog = null
        }
    }
    LaunchedEffect(enabled) {
        if (!enabled) {
            lifetime.dialog?.dismiss()
            lifetime.dialog = null
        }
    }
    fun write(text: String) {
        if (lifetime.active && latestEnabled) latestChanged(text)
    }
    fun show(dialog: Dialog) {
        if (!lifetime.active || !latestEnabled) return
        lifetime.dialog?.dismiss()
        lifetime.dialog = dialog
        dialog.setOnDismissListener { if (lifetime.dialog === dialog) lifetime.dialog = null }
        dialog.show()
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (label.isNotEmpty()) Text(label)
        OutlinedButton(enabled = enabled && (enableDate || enableTime),
            modifier = Modifier.heightIn(min = 48.dp), onClick = {
                val now = ZonedDateTime.now()
                val instant = runCatching { OffsetDateTime.parse(value).atZoneSameInstant(ZoneId.systemDefault()) }.getOrNull()
                val date = instant?.toLocalDate() ?: runCatching { LocalDate.parse(value) }.getOrDefault(now.toLocalDate())
                val time = instant?.toLocalTime() ?: runCatching { LocalTime.parse(value) }.getOrDefault(now.toLocalTime())
                fun pickTime(selectedDate: LocalDate?) {
                    show(TimePickerDialog(context, { _, hour, minute ->
                        val selectedTime = LocalTime.of(hour, minute)
                        if (selectedDate == null) write(selectedTime.format(DateTimeFormatter.ofPattern("HH:mm")))
                        else write(selectedDate.atTime(selectedTime).atZone(ZoneId.systemDefault()).toInstant().toString())
                    }, time.hour, time.minute, android.text.format.DateFormat.is24HourFormat(context)))
                }
                if (enableDate) {
                    show(DatePickerDialog(context, { _, year, month, day ->
                        if (!lifetime.active || !latestEnabled) return@DatePickerDialog
                        val selected = LocalDate.of(year, month + 1, day)
                        if (enableTime) pickTime(selected) else write(selected.toString())
                    }, date.year, date.monthValue - 1, date.dayOfMonth))
                } else pickTime(null)
            }) {
            Text(value.ifEmpty { context.getString(R.string.ui_pick_datetime) })
        }
        if (value.isNotEmpty()) TextButton(enabled = enabled, onClick = { write("") }) { Text(stringResource(R.string.ui_clear_value)) }
    }
}


private class PickerLifetime {
    var active = false
    var dialog: Dialog? = null
}
