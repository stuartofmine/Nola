package com.nola.data.local.db.entity

import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey

@Entity(
    tableName = "scheduled_actions",
    indices = [Index("localUserId")]
)
data class ScheduledActionEntity(
    @PrimaryKey val id: String,
    val localUserId: String,
    val name: String,
    val url: String,
    val format: String,
    val userPrompt: String? = null,
    val scheduleFrequency: String = "none",
    val scheduledWeekday: Int? = null,
    val scheduledTimeIso: String? = null,
    val timezone: String = "UTC",
    val enabled: Boolean = true,
    val backendSourceId: String? = null,
    val createdAt: Long = System.currentTimeMillis()
)

@Entity(
    tableName = "execution_results",
    indices = [Index("actionId")]
)
data class ExecutionResultEntity(
    @PrimaryKey val id: String,
    val actionId: String,
    val sourceName: String? = null,
    val sourceUrl: String? = null,
    val sourceFormat: String? = null,
    val status: String = "pending",
    val scheduledAt: Long? = null,
    val fetchedAt: Long? = null,
    val llmSummaryText: String? = null,
    val failureReason: String? = null,
    val eventsJson: String = "[]",
    val backendResultId: String? = null,
    val createdAt: Long = System.currentTimeMillis()
)

@Entity(
    tableName = "notifications",
    indices = [Index("isRead"), Index("createdAt")]
)
data class NotificationEntity(
    @PrimaryKey val id: String,
    val title: String,
    val body: String? = null,
    val source: String = "system",
    val notificationType: String = "message",
    val dedupeKey: String? = null,
    val payloadJson: String = "{}",
    val isRead: Boolean = false,
    val createdAt: Long = System.currentTimeMillis(),
    val readAt: Long? = null
)
