package com.nola.di

import android.content.Context
import androidx.room.Room
import com.nola.data.local.db.NolaDatabase
import com.nola.data.local.db.dao.ExecutionResultDao
import com.nola.data.local.db.dao.NotificationDao
import com.nola.data.local.db.dao.ScheduledActionDao
import com.nola.data.local.db.dao.AgentTraceDao
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object DatabaseModule {
    @Provides
    @Singleton
    fun provideDatabase(@ApplicationContext ctx: Context): NolaDatabase =
        Room.databaseBuilder(ctx, NolaDatabase::class.java, "nola.db").build()

    @Provides fun provideScheduledActionDao(db: NolaDatabase): ScheduledActionDao = db.scheduledActionDao()
    @Provides fun provideExecutionResultDao(db: NolaDatabase): ExecutionResultDao = db.executionResultDao()
    @Provides fun provideNotificationDao(db: NolaDatabase): NotificationDao = db.notificationDao()
    @Provides fun provideAgentTraceDao(db: NolaDatabase): AgentTraceDao = db.agentTraceDao()
}
