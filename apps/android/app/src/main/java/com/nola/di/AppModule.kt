package com.nola.di

import com.nola.llm.ModelService
import com.nola.llm.provider.PlaceholderProvider
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object AppModule {

    @Provides
    @Singleton
    fun provideModelService(placeholder: PlaceholderProvider): ModelService =
        ModelService(placeholder)
}
