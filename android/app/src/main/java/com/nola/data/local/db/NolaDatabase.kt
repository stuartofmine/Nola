package com.nola.data.local.db

import androidx.room.Database
import androidx.room.RoomDatabase
import com.nola.data.local.db.dao.ExecutionResultDao
import com.nola.data.local.db.dao.NotificationDao
import com.nola.data.local.db.dao.ScheduledActionDao
import com.nola.data.local.db.dao.AgentTraceDao
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.local.db.entity.AgentTraceEventEntity

@Database(
    entities = [
        ScheduledActionEntity::class,
        ExecutionResultEntity::class,
        NotificationEntity::class,
        AgentTraceEventEntity::class
    ],
    version = 1,
    exportSchema = false
)
abstract class NolaDatabase : RoomDatabase() {
    abstract fun scheduledActionDao(): ScheduledActionDao
    abstract fun executionResultDao(): ExecutionResultDao
    abstract fun notificationDao(): NotificationDao
    abstract fun agentTraceDao(): AgentTraceDao
}
