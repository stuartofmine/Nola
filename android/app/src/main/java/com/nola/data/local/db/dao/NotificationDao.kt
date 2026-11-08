package com.nola.data.local.db.dao

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import com.nola.data.local.db.entity.NotificationEntity

@Dao
interface NotificationDao {
    @Query("SELECT * FROM notifications ORDER BY createdAt DESC LIMIT :limit")
    suspend fun getRecent(limit: Int = 100): List<NotificationEntity>

    @Query("SELECT * FROM notifications WHERE isRead = 0 ORDER BY createdAt DESC")
    suspend fun getUnread(): List<NotificationEntity>

    @Query("SELECT COUNT(*) FROM notifications WHERE isRead = 0")
    suspend fun unreadCount(): Int

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(notification: NotificationEntity)

    @Query("UPDATE notifications SET isRead = 1, readAt = :now WHERE id = :id")
    suspend fun markRead(id: String, now: Long = System.currentTimeMillis())

    @Query("UPDATE notifications SET isRead = 1, readAt = :now")
    suspend fun markAllRead(now: Long = System.currentTimeMillis())
}
