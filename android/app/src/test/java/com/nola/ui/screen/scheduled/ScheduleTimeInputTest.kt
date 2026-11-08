package com.nola.ui.screen.scheduled

import java.time.LocalDateTime
import java.time.OffsetDateTime
import java.time.ZoneOffset
import org.junit.Assert.*
import org.junit.Test

class ScheduleTimeInputTest {
    @Test fun utcStoredInstantDisplaysInSelectedZoneAndRoundTrips() {
        val raw = "2026-10-09T12:30:00Z"
        assertEquals(LocalDateTime.of(2026, 10, 9, 20, 30), scheduleLocalDateTime(raw, "Asia/Shanghai"))
        val valid = validateScheduleTime("once", raw, "Asia/Shanghai") as ScheduleTimeValidation.Valid
        assertEquals(OffsetDateTime.parse(raw).toInstant(), OffsetDateTime.parse(valid.wireValue).toInstant())
        assertTrue(valid.wireValue.endsWith("+08:00"))
    }

    @Test fun storedOffsetSecondOccurrenceSurvivesEditingUnrelatedFields() {
        val raw = "2026-11-01T01:30:00-05:00"
        val valid = validateScheduleTime("once", raw, "America/New_York") as ScheduleTimeValidation.Valid
        assertEquals(OffsetDateTime.parse(raw).toInstant(), OffsetDateTime.parse(valid.wireValue).toInstant())
        assertTrue(valid.wireValue.endsWith("-05:00"))
    }

    @Test fun nonexistentSpringTimeIsRejectedRatherThanShifted() {
        assertEquals(ScheduleTimeValidation.Gap,
            validateScheduleTime("once", "2026-03-08T02:30", "America/New_York"))
    }

    @Test fun repeatedTimeRequiresChoiceAndBothChoicesResolveToDistinctInstants() {
        val result = validateScheduleTime("once", "2026-11-01T01:30", "America/New_York") as ScheduleTimeValidation.Overlap
        assertEquals(listOf(ZoneOffset.of("-04:00"), ZoneOffset.of("-05:00")), result.offsets)
        val values = result.offsets.map {
            (validateScheduleTime("once", "2026-11-01T01:30$it", "America/New_York") as ScheduleTimeValidation.Valid).wireValue
        }
        assertEquals(3600L, OffsetDateTime.parse(values[1]).toEpochSecond() - OffsetDateTime.parse(values[0]).toEpochSecond())
    }

    @Test fun timezoneChangePreservesDisplayedClockButRevalidatesNewZone() {
        val raw = "2026-03-08T07:30:00Z"
        val local = scheduleValueForZoneChange("once", raw, "Asia/Kolkata")
        assertEquals("2026-03-08T13:00", local)
        val valid = validateScheduleTime("once", local, "UTC") as ScheduleTimeValidation.Valid
        assertEquals("2026-03-08T13:00:00Z", valid.wireValue)
        assertEquals(ScheduleTimeValidation.Gap,
            validateScheduleTime("once", scheduleValueForZoneChange("once", "2026-03-08T02:30Z", "UTC"), "America/New_York"))
    }

    @Test fun recurringSchedulesAreDateIndependentWallTimes() {
        listOf("daily", "weekly").forEach { frequency ->
            assertEquals(ScheduleTimeValidation.Valid("02:30"), validateScheduleTime(frequency, "02:30", "America/New_York"))
            assertEquals("02:30", scheduleValueForZoneChange(frequency, "02:30", "America/New_York"))
        }
    }

    @Test fun malformedDateInvalidZoneAndTrailingGarbageAreRejected() {
        listOf("2026-02-30T09:00", "2026-10-09T09:00 trailing", "2026-10-09T25:00").forEach {
            assertEquals(ScheduleTimeValidation.Invalid, validateScheduleTime("once", it, "UTC"))
        }
        assertEquals(ScheduleTimeValidation.Invalid, validateScheduleTime("once", "2026-10-09T09:00", "Unknown/City"))
        assertEquals(ScheduleTimeValidation.Invalid, validateScheduleTime("daily", "09:00 trailing", "UTC"))
    }

    @Test fun fractionalOffsetAndNonHourDstAreNotAssumedToBeWholeHours() {
        assertEquals(ScheduleTimeValidation.Valid("2026-10-09T09:00:00+05:45"),
            validateScheduleTime("once", "2026-10-09T09:00", "Asia/Kathmandu"))
        assertEquals(ScheduleTimeValidation.Gap, validateScheduleTime("once", "2026-10-04T02:15", "Australia/Lord_Howe"))
        val overlap = validateScheduleTime("once", "2026-04-05T01:45", "Australia/Lord_Howe") as ScheduleTimeValidation.Overlap
        assertEquals(1800, overlap.offsets[0].totalSeconds - overlap.offsets[1].totalSeconds)
    }

    @Test fun unknownFrequencyIsNotReinterpretedAsOnce() {
        assertEquals(ScheduleTimeValidation.Invalid, validateScheduleTime("cron", "0 9 * * *", "UTC"))
    }
}
