package com.nola.work

import androidx.work.ListenableWorker
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.DataSourceDocumentInspector
import com.nola.ui.screen.scheduled.nextScheduledAt
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ScheduledActionAgentContractTest {
    @Test
    fun configuredSourceBecomesTypedReferenceContext() {
        val action = ScheduledActionEntity(
            id = "action-1",
            localUserId = "user-1",
            name = "Daily revenue",
            url = "https://example.com/revenue.csv",
            format = "csv",
            userPrompt = "Summarize material changes",
        )
        val source = DataSourceDocumentInspector.inspect(
            sourceId = "scheduled_source:action-1",
            location = action.url,
            contentType = "text/csv",
            rawContent = "day,revenue\nMonday,42",
            maximumAgeMinutes = 60,
            now = 1_700_000_000_000,
        )

        val component = scheduledActionReferenceComponent(action, source, "execution-1")

        assertEquals("web_retrieval", component.kind)
        assertEquals("scheduled_source:action-1:execution-1", component.id)
        assertEquals(action.url, component.source?.uri)
        assertEquals("text/csv", component.source?.mimeType)
        assertEquals("authorized", component.source?.authorization)
    }

    @Test
    fun scheduledSourceUsesExistingValidatedReaderContract() {
        val action = ScheduledActionEntity(
            id = "action-2",
            localUserId = "user-1",
            name = "Feed",
            url = "https://example.com/feed.json",
            format = "json",
        )

        val source = action.toDataSourceConfig()

        assertEquals("scheduled_source:action-2", source.id)
        assertEquals(action.url, source.location)
        assertEquals(60, source.maximumAgeMinutes)
    }

    @Test
    fun scheduledActionMimeTypeMapsAllSupportedFormatsAndFallbacks() {
        assertEquals("application/json", scheduledActionMimeType("json"))
        assertEquals("application/json", scheduledActionMimeType(" JSON "))
        assertEquals("text/csv", scheduledActionMimeType("csv"))
        assertEquals("application/rss+xml", scheduledActionMimeType("rss"))
        assertEquals("text/calendar", scheduledActionMimeType("icalendar"))
        assertEquals("text/calendar", scheduledActionMimeType("calendar"))
        assertEquals("text/html", scheduledActionMimeType("html"))
        assertEquals("text/plain", scheduledActionMimeType("url"))
        assertEquals("text/plain", scheduledActionMimeType("unknown"))
    }

    @Test
    fun nextScheduledAtHandlesOnceDailyWeeklyDstAndTimezones() {
        val now = ZonedDateTime.parse("2026-03-08T06:00:00Z").toInstant().toEpochMilli()

        val disabled = ScheduledActionEntity(
            id = "disabled",
            localUserId = "u",
            name = "Disabled",
            url = "https://example.test",
            format = "json",
            scheduleFrequency = "daily",
            scheduledTimeIso = "09:00",
            timezone = "UTC",
            enabled = false,
        )
        assertNull(nextScheduledAt(disabled, now))

        val onceFuture = ScheduledActionEntity(
            id = "once-future",
            localUserId = "u",
            name = "Once",
            url = "https://example.test",
            format = "json",
            scheduleFrequency = "once",
            scheduledTimeIso = "2026-03-08T09:00:00Z",
            timezone = "UTC",
        )
        assertEquals(
            ZonedDateTime.parse("2026-03-08T09:00:00Z").toInstant().toEpochMilli(),
            nextScheduledAt(onceFuture, now),
        )
        assertNull(nextScheduledAt(onceFuture, ZonedDateTime.parse("2026-03-08T09:00:01Z").toInstant().toEpochMilli()))

        // 2026-03-08 is US spring-forward DST day in America/New_York (06:00Z == 01:00 EST).
        // 08:30 EDT is 12:30Z (5.5h after 07:00Z? No: 02:00 EST jumps to 03:00 EDT, so 08:30 EDT == 12:30Z).
        val dailyNy = ScheduledActionEntity(
            id = "daily-ny",
            localUserId = "u",
            name = "Daily NY",
            url = "https://example.test",
            format = "json",
            scheduleFrequency = "daily",
            scheduledTimeIso = "08:30",
            timezone = "America/New_York",
        )
        val nySpring = nextScheduledAt(dailyNy, now)
        assertEquals(
            ZonedDateTime.parse("2026-03-08T08:30:00-04:00[America/New_York]").toInstant().toEpochMilli(),
            nySpring,
        )

        // Past wall time today rolls to next day; Tokyo has no DST.
        val dailyTokyo = dailyNy.copy(id = "daily-tyo", scheduledTimeIso = "07:00", timezone = "Asia/Tokyo")
        val tokyoNow = ZonedDateTime.parse("2026-03-08T08:00:00+09:00[Asia/Tokyo]").toInstant().toEpochMilli()
        assertEquals(
            ZonedDateTime.parse("2026-03-09T07:00:00+09:00[Asia/Tokyo]").toInstant().toEpochMilli(),
            nextScheduledAt(dailyTokyo, tokyoNow),
        )

        // Weekly ISO weekday 1 (Monday): 2026-03-08 is Sunday, so same week wraps or advances cleanly.
        val weekly = ScheduledActionEntity(
            id = "weekly-mon",
            localUserId = "u",
            name = "Weekly",
            url = "https://example.test",
            format = "json",
            scheduleFrequency = "weekly",
            scheduledWeekday = 1,
            scheduledTimeIso = "09:15",
            timezone = "UTC",
        )
        val mondayBefore = ZonedDateTime.parse("2026-03-09T08:00:00Z").toInstant().toEpochMilli()
        val mondayAfter = ZonedDateTime.parse("2026-03-09T10:00:00Z").toInstant().toEpochMilli()
        assertEquals(
            ZonedDateTime.parse("2026-03-09T09:15:00Z").toInstant().toEpochMilli(),
            nextScheduledAt(weekly, mondayBefore),
        )
        assertEquals(
            ZonedDateTime.parse("2026-03-16T09:15:00Z").toInstant().toEpochMilli(),
            nextScheduledAt(weekly, mondayAfter),
        )
    }

    @Test
    fun workerOutcomesAndReceiptPublishingEnforceAttentionRetryIdempotencyAndAutoDisable() {
        assertEquals(ScheduledActionOutcome.Success, outcomeForCanonicalAutomationKind("answered"))
        assertEquals(ScheduledActionOutcome.Success, outcomeForCanonicalAutomationKind("action_completed"))
        assertEquals(ScheduledActionOutcome.NeedsAttention, outcomeForCanonicalAutomationKind("incomplete"))
        assertEquals(ScheduledActionOutcome.NeedsAttention, outcomeForCanonicalAutomationKind("blocked"))
        assertEquals(ScheduledActionOutcome.AlreadyHandled, outcomeForCanonicalAutomationKind("delegation_accepted"))
        assertEquals(ScheduledActionOutcome.AlreadyHandled, outcomeForCanonicalAutomationKind("failed"))
        assertEquals(ScheduledActionOutcome.AlreadyHandled, outcomeForCanonicalAutomationKind("cancelled"))
        assertThrows(IllegalStateException::class.java) {
            outcomeForCanonicalAutomationKind("unrecognized_state")
        }

        assertEquals(ListenableWorker.Result.retry(), workResultForScheduledOutcomes(listOf(ScheduledActionOutcome.Success, ScheduledActionOutcome.Deferred)))
        assertEquals(
            ListenableWorker.Result.success(),
            workResultForScheduledOutcomes(listOf(ScheduledActionOutcome.Success, ScheduledActionOutcome.NeedsAttention)),
        )
        assertEquals(
            ListenableWorker.Result.success(),
            workResultForScheduledOutcomes(listOf(ScheduledActionOutcome.Success, ScheduledActionOutcome.AlreadyHandled)),
        )


        assertTrue(shouldAutoDisableAfterReceipt("none"))
        assertTrue(shouldAutoDisableAfterReceipt("once"))
        assertFalse(shouldAutoDisableAfterReceipt("daily"))
        assertFalse(shouldAutoDisableAfterReceipt("weekly"))

        val action = ScheduledActionEntity(
            id = "action-receipt",
            localUserId = "u",
            name = "Morning Digest",
            url = "https://example.test/digest.json",
            format = "json",
            scheduleFrequency = "none",
        )
        val firstReceipt = buildReceiptExecutionResult(
            existing = null,
            action = action,
            occurrenceId = "occ-42",
            kind = "answered",
            summaryText = "Summary line", sessionId = "session-1", completedAtMs = 123456L,
        )
        assertEquals("scheduled-result:occ-42", firstReceipt.id)
        assertEquals("answered", firstReceipt.status)
        assertEquals("Summary line", firstReceipt.llmSummaryText)

        // Reconciling the same occurrence reuses the exact stable row ID instead of inserting duplicates.
        val replayed = buildReceiptExecutionResult(
            existing = firstReceipt.copy(scheduledAt = 12345L, failureReason = "permission_required", eventsJson = """{"pending_permission":{}}"""),
            action = action,
            occurrenceId = "occ-42",
            kind = "answered",
            summaryText = "Summary line", sessionId = "session-1", completedAtMs = 123456L,
        )
        assertEquals(firstReceipt.id, replayed.id)
        assertEquals(12345L, replayed.scheduledAt)
        assertNotNull(replayed.llmSummaryText)
        assertNull(replayed.failureReason)
        assertFalse(replayed.eventsJson.contains("pending_permission"))
        assertEquals(123456L, replayed.journalCompletedAt)
        assertEquals("session-1", replayed.journalSessionId)
    }
}
