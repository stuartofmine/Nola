package com.nola.data.health

import android.content.Context
import androidx.health.connect.client.HealthConnectClient
import androidx.health.connect.client.PermissionController
import androidx.health.connect.client.aggregate.AggregateMetric
import androidx.health.connect.client.permission.HealthPermission
import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.HeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.request.AggregateRequest
import androidx.health.connect.client.request.ReadRecordsRequest
import androidx.health.connect.client.time.TimeRangeFilter
import dagger.hilt.android.qualifiers.ApplicationContext
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.temporal.ChronoUnit
import javax.inject.Inject
import javax.inject.Singleton

data class AndroidHealthSummary(
    val availability: AndroidHealthAvailability,
    val permissionState: AndroidHealthPermissionState,
    val missingPermissionCount: Int = 0,
    val days: Int = 7,
    val activityDate: Instant? = null,
    val totalSteps: Long? = null,
    val averageStepsPerDay: Int? = null,
    val averageHeartRateBpm: Int? = null,
    val averageSleepHoursPerDay: Double? = null,
    val latestWorkoutType: String? = null,
    val latestWorkoutMinutes: Int? = null,
    val latestWorkoutDaysAgo: Int? = null,
    val updatedAt: Instant? = null,
    val error: String? = null
) {
    val hasHealthData: Boolean
        get() = totalSteps != null ||
            averageHeartRateBpm != null ||
            averageSleepHoursPerDay != null ||
            latestWorkoutType != null
}

enum class AndroidHealthAvailability {
    Available,
    ProviderUpdateRequired,
    Unavailable
}

enum class AndroidHealthPermissionState {
    Granted,
    Missing,
    Unavailable
}

@Singleton
class AndroidHealthDataSource @Inject constructor(
    @ApplicationContext private val context: Context
) {
    companion object {
        val readPermissions: Set<String> = setOf(
            HealthPermission.getReadPermission(StepsRecord::class),
            HealthPermission.getReadPermission(HeartRateRecord::class),
            HealthPermission.getReadPermission(SleepSessionRecord::class),
            HealthPermission.getReadPermission(ExerciseSessionRecord::class)
        )

        fun permissionContract() =
            PermissionController.createRequestPermissionResultContract()
    }

    private val client: HealthConnectClient by lazy {
        HealthConnectClient.getOrCreate(context)
    }

    fun availability(): AndroidHealthAvailability {
        return when (HealthConnectClient.getSdkStatus(context)) {
            HealthConnectClient.SDK_AVAILABLE -> AndroidHealthAvailability.Available
            HealthConnectClient.SDK_UNAVAILABLE_PROVIDER_UPDATE_REQUIRED -> AndroidHealthAvailability.ProviderUpdateRequired
            else -> AndroidHealthAvailability.Unavailable
        }
    }

    suspend fun hasReadPermission(): Boolean {
        if (availability() != AndroidHealthAvailability.Available) return false
        val granted = client.permissionController.getGrantedPermissions()
        return readPermissions.any(granted::contains)
    }

    suspend fun readSummary(days: Int = 7): AndroidHealthSummary {
        val currentAvailability = availability()
        if (currentAvailability != AndroidHealthAvailability.Available) {
            return AndroidHealthSummary(
                availability = currentAvailability,
                permissionState = AndroidHealthPermissionState.Unavailable,
                days = days
            )
        }

        val grantedPermissions = client.permissionController.getGrantedPermissions()
        val grantedReadPermissions = readPermissions.intersect(grantedPermissions)
        if (grantedReadPermissions.isEmpty()) {
            return AndroidHealthSummary(
                availability = currentAvailability,
                permissionState = AndroidHealthPermissionState.Missing,
                missingPermissionCount = readPermissions.size,
                days = days
            )
        }
        val canReadSteps = HealthPermission.getReadPermission(StepsRecord::class) in grantedPermissions
        val canReadHeartRate = HealthPermission.getReadPermission(HeartRateRecord::class) in grantedPermissions
        val canReadSleep = HealthPermission.getReadPermission(SleepSessionRecord::class) in grantedPermissions
        val canReadExercise = HealthPermission.getReadPermission(ExerciseSessionRecord::class) in grantedPermissions

        val safeDays = days.coerceAtLeast(1)
        val end = Instant.now()
        val zone = ZoneId.systemDefault()
        val today = LocalDate.now(zone)
        val recentStart = end.minus(safeDays.toLong(), ChronoUnit.DAYS)
        val recentRange = TimeRangeFilter.between(recentStart, end)

        return try {
            var activityDate: Instant? = null
            var activitySteps: Long? = null
            for (offset in 0 until safeDays) {
                if (!canReadSteps) break
                val date = today.minusDays(offset.toLong())
                val dayStart = date.atStartOfDay(zone).toInstant()
                val dayEnd = if (offset == 0) end else date.plusDays(1).atStartOfDay(zone).toInstant()
                val steps = aggregateLong(TimeRangeFilter.between(dayStart, dayEnd), StepsRecord.COUNT_TOTAL)
                if (steps != null) {
                    activityDate = dayStart
                    activitySteps = steps
                    break
                }
            }
            val latestHeartRate = if (canReadHeartRate) readLatestHeartRate(recentRange) else null
            val sleepStart = end.minus(36, ChronoUnit.HOURS)
            val sleepHours = if (canReadSleep) {
                readLatestSleepHours(TimeRangeFilter.between(sleepStart, end))
            } else null
            val latestWorkout = if (canReadExercise) readLatestWorkout(recentRange, end) else null

            AndroidHealthSummary(
                availability = currentAvailability,
                permissionState = AndroidHealthPermissionState.Granted,
                missingPermissionCount = readPermissions.size - grantedReadPermissions.size,
                days = safeDays,
                activityDate = activityDate,
                totalSteps = activitySteps,
                averageStepsPerDay = activitySteps?.toInt(),
                averageHeartRateBpm = latestHeartRate,
                averageSleepHoursPerDay = sleepHours,
                latestWorkoutType = latestWorkout?.type,
                latestWorkoutMinutes = latestWorkout?.minutes,
                latestWorkoutDaysAgo = latestWorkout?.daysAgo,
                updatedAt = end
            )
        } catch (error: Exception) {
            AndroidHealthSummary(
                availability = currentAvailability,
                permissionState = AndroidHealthPermissionState.Granted,
                missingPermissionCount = readPermissions.size - grantedReadPermissions.size,
                days = safeDays,
                updatedAt = end,
                error = error.localizedMessage ?: error.javaClass.simpleName
            )
        }
    }

    private suspend fun aggregateLong(
        range: TimeRangeFilter,
        metric: AggregateMetric<Long>
    ): Long? {
        val response = client.aggregate(AggregateRequest(metrics = setOf(metric), timeRangeFilter = range))
        return response[metric]?.takeIf { it > 0L }
    }

    private suspend fun readLatestHeartRate(range: TimeRangeFilter): Int? {
        return client.readRecords(
            ReadRecordsRequest(
                recordType = HeartRateRecord::class,
                timeRangeFilter = range
            )
        ).records
            .asSequence()
            .flatMap { it.samples.asSequence() }
            .maxByOrNull { it.time }
            ?.beatsPerMinute
            ?.toInt()
    }

    private suspend fun readLatestSleepHours(range: TimeRangeFilter): Double? {
        val records = client.readRecords(
            ReadRecordsRequest(
                recordType = SleepSessionRecord::class,
                timeRangeFilter = range
            )
        ).records
        val latest = records.maxByOrNull { it.endTime } ?: return null
        val seconds = Duration.between(latest.startTime, latest.endTime).seconds
        return seconds.takeIf { it > 0L }?.let { it / 3600.0 }
    }

    private suspend fun readLatestWorkout(range: TimeRangeFilter, now: Instant): WorkoutSummary? {
        val record = client.readRecords(
            ReadRecordsRequest(
                recordType = ExerciseSessionRecord::class,
                timeRangeFilter = range
            )
        ).records.maxByOrNull { it.endTime }
        return record?.let {
            val minutes = Duration.between(it.startTime, it.endTime).toMinutes().toInt().coerceAtLeast(1)
            WorkoutSummary(
                type = exerciseTypeName(it.exerciseType),
                minutes = minutes,
                daysAgo = Duration.between(it.endTime, now).toDays().toInt().coerceAtLeast(0)
            )
        }
    }

    private fun exerciseTypeName(type: Int): String {
        return when (type) {
            ExerciseSessionRecord.EXERCISE_TYPE_RUNNING -> context.getString(com.nola.R.string.data_workout_running)
            ExerciseSessionRecord.EXERCISE_TYPE_WALKING -> context.getString(com.nola.R.string.data_workout_walking)
            ExerciseSessionRecord.EXERCISE_TYPE_BIKING -> context.getString(com.nola.R.string.data_workout_cycling)
            ExerciseSessionRecord.EXERCISE_TYPE_SWIMMING_OPEN_WATER,
            ExerciseSessionRecord.EXERCISE_TYPE_SWIMMING_POOL -> context.getString(com.nola.R.string.data_workout_swimming)
            ExerciseSessionRecord.EXERCISE_TYPE_YOGA -> context.getString(com.nola.R.string.data_workout_yoga)
            ExerciseSessionRecord.EXERCISE_TYPE_HIGH_INTENSITY_INTERVAL_TRAINING -> context.getString(com.nola.R.string.data_workout_hiit)
            ExerciseSessionRecord.EXERCISE_TYPE_STRENGTH_TRAINING -> context.getString(com.nola.R.string.data_workout_strength)
            else -> context.getString(com.nola.R.string.data_workout_generic)
        }
    }

    private data class WorkoutSummary(
        val type: String,
        val minutes: Int,
        val daysAgo: Int
    )
}
