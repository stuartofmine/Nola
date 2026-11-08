package com.nola.ui.screen.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.core.IntelligenceEvent
import com.nola.core.IntelligenceIngestRequest
import com.nola.core.IntelligenceMemoryCandidate
import com.nola.core.PersonalIntelligenceCore
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthSummary
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.time.ZoneId
import java.util.UUID
import javax.inject.Inject

data class HealthSummaryUiState(
    val summary: AndroidHealthSummary? = null,
    val isLoading: Boolean = false
)

@HiltViewModel
class HealthSummaryViewModel @Inject constructor(
    private val healthDataSource: AndroidHealthDataSource
) : ViewModel() {
    private val _uiState = MutableStateFlow(HealthSummaryUiState())
    val uiState: StateFlow<HealthSummaryUiState> = _uiState.asStateFlow()

    fun refresh() {
        if (_uiState.value.isLoading) return
        viewModelScope.launch {
            _uiState.update { it.copy(isLoading = true) }
            val summary = healthDataSource.readSummary()
            _uiState.update { it.copy(summary = summary, isLoading = false) }
            recordPersonalIntelligence(summary)
        }
    }

    fun refreshAfterPermissionResult() {
        refresh()
    }

    private fun recordPersonalIntelligence(summary: AndroidHealthSummary) {
        if (!summary.hasHealthData) return
        val day = summary.activityDate
            ?.atZone(ZoneId.systemDefault())
            ?.toLocalDate()
            ?.toString()
            ?: "latest"
        val observedAt = summary.updatedAt?.toString().orEmpty()
        val eventId = UUID.randomUUID().toString()
        val steps = summary.totalSteps ?: 0L
        val heartRate = summary.averageHeartRateBpm
        val sleep = summary.averageSleepHoursPerDay
        val summaryText = buildString {
            append("$day 健康 / health: $steps 步 / steps")
            heartRate?.let { append(", $it bpm") }
            sleep?.let { append(", ${"%.1f".format(it)} h sleep") }
        }
        val candidates = buildList {
            add(
                IntelligenceMemoryCandidate(
                    semantic_key = "health.activity.$day",
                    kind = "episodic",
                    statement = summaryText,
                    confidence = 1.0,
                    source_event_ids = listOf(eventId),
                    observed_at = observedAt,
                    tags = listOf("health", "健康", "activity", "活动", "steps", "步数")
                )
            )
            heartRate?.let {
                add(
                    IntelligenceMemoryCandidate(
                        semantic_key = "health.heart_rate.latest",
                        kind = "episodic",
                        statement = "最近心率 / latest heart rate: $it bpm",
                        confidence = 1.0,
                        source_event_ids = listOf(eventId),
                        observed_at = observedAt,
                        tags = listOf("health", "健康", "heart", "心率")
                    )
                )
            }
            sleep?.let {
                add(
                    IntelligenceMemoryCandidate(
                        semantic_key = "health.sleep.latest",
                        kind = "episodic",
                        statement = "最近睡眠 / latest sleep: ${"%.1f".format(it)} hours",
                        confidence = 1.0,
                        source_event_ids = listOf(eventId),
                        observed_at = observedAt,
                        tags = listOf("health", "健康", "sleep", "睡眠")
                    )
                )
            }
            if (!summary.latestWorkoutType.isNullOrBlank()) {
                add(
                    IntelligenceMemoryCandidate(
                        semantic_key = "health.workout.latest",
                        kind = "episodic",
                        statement = "最近训练 / latest workout: ${summary.latestWorkoutType}, ${summary.latestWorkoutMinutes ?: 0} min",
                        confidence = 1.0,
                        source_event_ids = listOf(eventId),
                        observed_at = observedAt,
                        tags = listOf("health", "健康", "workout", "训练")
                    )
                )
            }
        }
        viewModelScope.launch(Dispatchers.IO) {
            PersonalIntelligenceCore.ingest(
                IntelligenceIngestRequest(
                    events = listOf(
                        IntelligenceEvent(
                            id = eventId,
                            source = "health_connect",
                            kind = "health.snapshot.updated",
                            occurred_at = observedAt,
                            observed_at = observedAt,
                            subject = day,
                            summary = summaryText,
                            importance = 0.64,
                            confidence = 1.0,
                            tags = listOf("health", "健康", "activity", "活动"),
                            dedupe_key = "health:$day:$steps:${heartRate ?: 0}:${sleep ?: 0.0}"
                        )
                    ),
                    memory_candidates = candidates
                )
            )
        }
    }
}
