package com.nola.ui.screen.scheduled

import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/** Editor validation only; recurrence execution remains owned by the scheduler. */
internal sealed interface ScheduleTimeValidation {
    data class Valid(val wireValue: String) : ScheduleTimeValidation
    data object Invalid : ScheduleTimeValidation
    data object Gap : ScheduleTimeValidation
    data class Overlap(val offsets: List<ZoneOffset>) : ScheduleTimeValidation
}

internal fun scheduleLocalDateTime(value: String, zone: String): LocalDateTime? = runCatching {
    val zoneId = ZoneId.of(zone)
    runCatching { OffsetDateTime.parse(value).atZoneSameInstant(zoneId).toLocalDateTime() }
        .getOrElse { LocalDateTime.parse(value.replace(' ', 'T')) }
}.getOrNull()

internal fun scheduleLocalTime(value: String, zone: String): LocalTime? =
    runCatching { LocalTime.parse(value) }.getOrNull()
        ?: scheduleLocalDateTime(value, zone)?.toLocalTime()

internal fun validateScheduleTime(frequency: String, value: String, zone: String): ScheduleTimeValidation {
    val zoneId = runCatching { ZoneId.of(zone) }.getOrNull() ?: return ScheduleTimeValidation.Invalid
    if (frequency in setOf("daily", "weekly")) {
        // Recurring schedules are wall times, never attach today's date or offset.
        val time = runCatching { LocalTime.parse(value) }.getOrNull() ?: return ScheduleTimeValidation.Invalid
        return ScheduleTimeValidation.Valid(time.format(DateTimeFormatter.ofPattern("HH:mm")))
    }
    if (frequency != "once") return ScheduleTimeValidation.Invalid
    val local = scheduleLocalDateTime(value, zone) ?: return ScheduleTimeValidation.Invalid
    val offsets = zoneId.rules.getValidOffsets(local)
    if (offsets.isEmpty()) return ScheduleTimeValidation.Gap
    val existing = runCatching { OffsetDateTime.parse(value).atZoneSameInstant(zoneId).offset }.getOrNull()
    val offset = existing?.takeIf { it in offsets } ?: offsets.singleOrNull()
        ?: return ScheduleTimeValidation.Overlap(offsets)
    return ScheduleTimeValidation.Valid(local.atOffset(offset).format(DateTimeFormatter.ISO_OFFSET_DATE_TIME))
}

/** Changing zone preserves the displayed clock/date, not the old instant. */
internal fun scheduleValueForZoneChange(frequency: String, value: String, oldZone: String): String =
    if (frequency == "once") scheduleLocalDateTime(value, oldZone)?.toString() ?: value else value
