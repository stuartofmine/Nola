package com.nola.di

import com.nola.data.repository.AuthRepository
import com.nola.data.repository.AuthRepositoryImpl
import com.nola.data.repository.ChatRepository
import com.nola.data.repository.ChatRepositoryImpl
import com.nola.data.repository.ContextRepository
import com.nola.data.repository.ContextRepositoryImpl
import com.nola.data.repository.ConversationRepository
import com.nola.data.repository.ConversationRepositoryImpl
import com.nola.data.repository.ModelRepository
import com.nola.data.repository.ModelRepositoryImpl
import com.nola.data.repository.NotificationRepository
import com.nola.data.repository.NotificationRepositoryImpl
import com.nola.data.repository.ScheduledActionRepository
import com.nola.data.repository.ScheduledActionRepositoryImpl
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class RepositoryModule {

    @Binds @Singleton
    abstract fun bindAuthRepo(impl: AuthRepositoryImpl): AuthRepository

    @Binds @Singleton
    abstract fun bindConversationRepo(impl: ConversationRepositoryImpl): ConversationRepository

    @Binds @Singleton
    abstract fun bindChatRepo(impl: ChatRepositoryImpl): ChatRepository

    @Binds @Singleton
    abstract fun bindContextRepo(impl: ContextRepositoryImpl): ContextRepository

    @Binds @Singleton
    abstract fun bindModelRepo(impl: ModelRepositoryImpl): ModelRepository

    @Binds @Singleton
    abstract fun bindScheduledActionRepo(impl: ScheduledActionRepositoryImpl): ScheduledActionRepository

    @Binds @Singleton
    abstract fun bindNotificationRepo(impl: NotificationRepositoryImpl): NotificationRepository
}
