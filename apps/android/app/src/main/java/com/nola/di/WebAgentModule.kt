package com.nola.di

import com.nola.llm.provider.ResourceWebAgentSummaryTextSource
import com.nola.llm.provider.WebAgentSummaryTextSource
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class WebAgentModule {
    @Binds
    @Singleton
    abstract fun bindWebAgentSummaryTextSource(
        impl: ResourceWebAgentSummaryTextSource
    ): WebAgentSummaryTextSource
}
