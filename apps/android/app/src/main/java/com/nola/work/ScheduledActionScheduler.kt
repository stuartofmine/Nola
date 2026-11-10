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
        enqueue(action, ExistingWorkPolicy.APPEND_OR_REPLACE)
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
        workManager.enqueueUniqueWork(uniqueWorkName(action.id), policy, request)
    }

    fun cancel(actionId: String) {
        workManager.cancelUniqueWork(uniqueWorkName(actionId))
        workManager.cancelAllWorkByTag(actionTag(actionId))
    }

    fun cancelAll() {
        workManager.cancelAllWorkByTag(TAG)
    }

    private fun nextOccurrence(action: ScheduledActionEntity): ZonedDateTime {
        val zone=ZoneId.of(action.timezone.ifBlank {"UTC"})
        val now=Instant.now().toEpochMilli()
        val result=application.executeQuery(kotlinx.serialization.json.buildJsonObject {
            put("type","query_automation_occurrences");put("input",kotlinx.serialization.json.buildJsonObject {
                put("automation_id",action.id);put("timezone",zone.id);put("frequency",action.scheduleFrequency)
                put("time",action.scheduledTimeIso?.let({ kotlinx.serialization.json.JsonPrimitive(it) }) ?: kotlinx.serialization.json.JsonNull)
                put("weekday",action.scheduledWeekday?.let({ kotlinx.serialization.json.JsonPrimitive(it) }) ?: kotlinx.serialization.json.JsonNull)
                put("once_at_ms",parseOnceTarget(action.scheduledTimeIso,zone)?.toInstant()?.toEpochMilli()?.let({ kotlinx.serialization.json.JsonPrimitive(it) }) ?: kotlinx.serialization.json.JsonNull)
                put("after_ms",if(action.scheduleFrequency=="once") (parseOnceTarget(action.scheduledTimeIso,zone)?.toInstant()?.toEpochMilli() ?: error("Invalid one-time date"))-1 else now)
                put("through_ms",if(action.scheduleFrequency=="once") maxOf(now,parseOnceTarget(action.scheduledTimeIso,zone)?.toInstant()?.toEpochMilli() ?: now) else now+8*86_400_000L)
                put("limit",1)
            })
        }).jsonObject
        val occurrence=result.getValue("occurrences").jsonArray.firstOrNull() ?: error("The schedule has no future occurrence")
        return Instant.ofEpochMilli(occurrence.jsonObject.getValue("scheduled_for_ms").jsonPrimitive.long).atZone(zone)
    }

    private fun parseOnceTarget(value: String?, zone: ZoneId): ZonedDateTime? {
        if (value.isNullOrBlank()) return null
        return runCatching { ZonedDateTime.parse(value).withZoneSameInstant(zone) }.getOrNull()
            ?: runCatching { OffsetDateTime.parse(value).atZoneSameInstant(zone) }.getOrNull()
            ?: runCatching { LocalDateTime.parse(value).atZone(zone) }.getOrNull()
    }

    private fun parseTime(value: String?): LocalTime? {
        if (value.isNullOrBlank()) return null
        return try {
            LocalTime.parse(value)
        } catch (_: DateTimeParseException) {
            null
        }
    }

    companion object {
        private const val TAG = "scheduled_actions"

        fun uniqueWorkName(actionId: String): String = "scheduled_action_$actionId"
        fun actionTag(actionId: String): String = "scheduled_action_id_$actionId"
    }
}
