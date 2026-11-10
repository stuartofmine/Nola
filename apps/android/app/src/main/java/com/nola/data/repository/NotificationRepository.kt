package com.nola.data.repository

import com.nola.data.local.db.dao.NotificationDao
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.remote.api.BackendApiService
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.time.Instant
import javax.inject.Inject
import javax.inject.Singleton

interface NotificationRepository {
    suspend fun getNotifications(limit: Int = 100): List<NotificationEntity>
    suspend fun getUnread(): List<NotificationEntity>
    suspend fun unreadCount(): Int
    suspend fun add(notification: NotificationEntity)
    suspend fun markRead(id: String)
    suspend fun markAllRead()
    suspend fun syncFromBackend()
}

@Singleton
class NotificationRepositoryImpl @Inject constructor(
    private val notificationDao: NotificationDao,
    private val api: BackendApiService
) : NotificationRepository {
    private val json = Json { prettyPrint = false }

    override suspend fun getNotifications(limit: Int): List<NotificationEntity> =
        notificationDao.getRecent(limit)

    override suspend fun getUnread(): List<NotificationEntity> =
        notificationDao.getUnread()

    override suspend fun unreadCount(): Int =
        notificationDao.unreadCount()

    override suspend fun add(notification: NotificationEntity) {
        notificationDao.upsert(notification)
    }

    override suspend fun markRead(id: String) {
        notificationDao.markRead(id)
        try {
            api.markNotificationRead(id)
        } catch (_: Exception) { }
    }

    override suspend fun markAllRead() {
        notificationDao.markAllRead()
        try {
            api.markAllNotificationsRead()
        } catch (_: Exception) { }
    }

    override suspend fun syncFromBackend() {
        try {
            val response = api.getNotifications(limit = 100)
            response.notifications.forEach { dto ->
                val now = System.currentTimeMillis()
                notificationDao.upsert(NotificationEntity(
                    id = dto.id,
                    title = dto.title,
                    body = dto.body,
                    source = dto.source ?: "system",
                    notificationType = dto.type ?: "message",
                    dedupeKey = dto.dedupeKey,
                    payloadJson = dto.payload?.let { json.encodeToString(it) } ?: "{}",
                    isRead = dto.isRead,
                    createdAt = parseTimestamp(dto.createdAt) ?: now,
                    readAt = parseTimestamp(dto.readAt)
                ))
            }
        } catch (_: Exception) { }
    }

    private fun parseTimestamp(value: String?): Long? {
        if (value.isNullOrBlank()) return null
        return runCatching { Instant.parse(value).toEpochMilli() }
            .getOrElse { value.toLongOrNull() }
    }
}
