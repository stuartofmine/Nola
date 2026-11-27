package com.nola.work

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import com.nola.data.local.db.entity.ScheduledActionEntity
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.serialization.json.*
import java.time.DayOfWeek
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeParseException
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ScheduledActionScheduler @Inject constructor(
    @ApplicationContext private val context: Context,
    private val application:com.nola.data.local.AndroidApplicationRuntime,
) {
    private val workManager: WorkManager get() = WorkManager.getInstance(context)

    fun schedule(action: ScheduledActionEntity) {
        cancel(action.id)
        if (!action.enabled) return
        enqueue(action, ExistingWorkPolicy.REPLACE)
    }

    fun scheduleNext(action: ScheduledActionEntity) {
        if (!action.enabled || action.scheduleFrequency !in setOf("daily", "weekly")) return
        enqueue(action, ExistingWorkPolicy.KEEP)
    }

    private fun enqueue(action: ScheduledActionEntity, policy: ExistingWorkPolicy) {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .build()
        val occurrence = nextOccurrence(action)
        val occurrenceMillis = occurrence.toInstant().toEpochMilli()
        val occurrenceID = "${action.id}:$occurrenceMillis"
        val inputData = workDataOf(
            ScheduledActionWorker.KEY_ACTION_ID to action.id,
            ScheduledActionWorker.KEY_OCCURRENCE_ID to occurrenceID,
            ScheduledActionWorker.KEY_SCHEDULED_AT to occurrenceMillis,
        )
        val delay = Duration.between(Instant.now(), occurrence.toInstant())
            .coerceAtLeast(Duration.ZERO)
        val request = OneTimeWorkRequestBuilder<ScheduledActionWorker>()
            .setConstraints(constraints)
            .setInputData(inputData)
            .setInitialDelay(delay)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
            .addTag(TAG)
            .addTag(actionTag(action.id))
            .build()
        val name = if (policy == ExistingWorkPolicy.KEEP) "scheduled_occurrence_$occurrenceID" else uniqueWorkName(action.id)
        workManager.enqueueUniqueWork(name, policy, request)
    }

    fun reconcileCompleted(actionId: String, occurrence: String) {
        val request = OneTimeWorkRequestBuilder<ScheduledActionWorker>()
            .setInputData(workDataOf(ScheduledActionWorker.KEY_ACTION_ID to actionId,
                ScheduledActionWorker.KEY_OCCURRENCE_ID to occurrence,
                ScheduledActionWorker.KEY_RECONCILE_ONLY to true))
            .build()
        workManager.enqueueUniqueWork("scheduled_receipt_$occurrence", ExistingWorkPolicy.KEEP, request)
    }

    fun cancel(actionId: String) {
        workManager.cancelUniqueWork(uniqueWorkName(actionId))
        workManager.cancelAllWorkByTag(actionTag(actionId))
    }

    fun cancelAll() {
        workManager.cancelAllWorkByTag(TAG)
    }

    private fun nextOccurrence(action: ScheduledActionEntity): ZonedDateTime {
        val zone = ZoneId.of(action.timezone.ifBlank { "UTC" })
        val nowInstant = Instant.now()
        val now = nowInstant.toEpochMilli()
        val onceTarget = parseOnceTarget(action.scheduledTimeIso, zone)
        val result = application.executeQuery(kotlinx.serialization.json.buildJsonObject {
            put("type", "query_automation_occurrences")
            put("input", kotlinx.serialization.json.buildJsonObject {
                put("automation_id", action.id)
                put("timezone", zone.id)
                put("frequency", action.scheduleFrequency)
                put("time", action.scheduledTimeIso?.let { kotlinx.serialization.json.JsonPrimitive(it) } ?: kotlinx.serialization.json.JsonNull)
                put("weekday", action.scheduledWeekday?.let { kotlinx.serialization.json.JsonPrimitive(it) } ?: kotlinx.serialization.json.JsonNull)
                put("once_at_ms", onceTarget?.toInstant()?.toEpochMilli()?.let { kotlinx.serialization.json.JsonPrimitive(it) } ?: kotlinx.serialization.json.JsonNull)
                put("after_ms", if (action.scheduleFrequency == "once") (onceTarget?.toInstant()?.toEpochMilli() ?: error("Invalid one-time date")) - 1 else now)
                put("through_ms", if (action.scheduleFrequency == "once") maxOf(now, onceTarget?.toInstant()?.toEpochMilli() ?: now) else now + 8 * 86_400_000L)
                put("limit", 1)
            })
        }).jsonObject
        val queried = result["occurrences"]?.jsonArray?.firstOrNull()
            ?.jsonObject?.get("scheduled_for_ms")?.jsonPrimitive?.longOrNull
        if (queried != null && queried > now) {
            return Instant.ofEpochMilli(queried).atZone(zone)
        }
        return computeNextOccurrenceLocally(action, zone, nowInstant)
            ?: error("The schedule has no future occurrence")
    }

    private fun parseTime(value: String?): LocalTime? = parseLocalScheduleTime(value)

    companion object {
        private const val TAG = "scheduled_actions"

        fun uniqueWorkName(actionId: String): String = "scheduled_action_$actionId"
        fun actionTag(actionId: String): String = "scheduled_action_id_$actionId"
    }
}

internal fun computeNextOccurrenceLocally(
    action: ScheduledActionEntity,
    zone: ZoneId,
    nowInstant: Instant = Instant.now(),
): ZonedDateTime? {
    val nowZoned = nowInstant.atZone(zone)
    return when (action.scheduleFrequency) {
        "once" -> parseOnceTarget(action.scheduledTimeIso, zone)?.takeIf { it.toInstant().isAfter(nowInstant) }
        "daily" -> {
            val time = parseScheduleClock(action.scheduledTimeIso) ?: return null
            var candidate = ZonedDateTime.of(nowZoned.toLocalDate(), time, zone)
            if (!candidate.toInstant().isAfter(nowInstant)) {
                candidate = candidate.plusDays(1)
            }
            candidate
        }
        "weekly" -> {
            val time = parseScheduleClock(action.scheduledTimeIso) ?: return null
            val isoWeekday = action.scheduledWeekday?.takeIf { it in 1..7 } ?: return null
            val targetDay = DayOfWeek.of(isoWeekday)
            val daysAhead = (targetDay.value - nowZoned.dayOfWeek.value + 7) % 7
            var candidate = ZonedDateTime.of(nowZoned.toLocalDate().plusDays(daysAhead.toLong()), time, zone)
            if (!candidate.toInstant().isAfter(nowInstant)) {
                candidate = candidate.plusWeeks(1)
            }
            candidate
        }
        else -> null
    }
}

private fun parseOnceTarget(value: String?, zone: ZoneId): ZonedDateTime? {
    if (value.isNullOrBlank()) return null
    return runCatching { ZonedDateTime.parse(value).withZoneSameInstant(zone) }.getOrNull()
        ?: runCatching { OffsetDateTime.parse(value).atZoneSameInstant(zone) }.getOrNull()
        ?: runCatching { LocalDateTime.parse(value).atZone(zone) }.getOrNull()
}

private fun parseLocalScheduleTime(value: String?): LocalTime? {
    if (value.isNullOrBlank()) return null
    return try {
        LocalTime.parse(value)
    } catch (_: DateTimeParseException) {
        null
    }
}

private fun parseScheduleClock(value: String?): LocalTime? {
    parseLocalScheduleTime(value)?.let { return it }
    val match = Regex("""(\d{1,2}):(\d{2})""").findAll(value.orEmpty()).lastOrNull() ?: return null
    val hour = match.groupValues[1].toIntOrNull()?.takeIf { it in 0..23 } ?: return null
    val minute = match.groupValues[2].toIntOrNull()?.takeIf { it in 0..59 } ?: return null
    return LocalTime.of(hour, minute)
}
