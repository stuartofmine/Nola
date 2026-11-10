package com.nola.di

import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.remote.api.BackendApiService
import com.nola.data.remote.api.BackendApiServiceImpl
import com.nola.data.remote.api.WebHostApiService
import com.nola.data.remote.api.WebHostApiServiceImpl
import com.nola.data.remote.api.WebHostHttpClientFactory
import com.nola.data.remote.sse.SseClient
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.logging.LogLevel
import io.ktor.client.plugins.logging.Logging
import io.ktor.serialization.kotlinx.json.json
import kotlinx.serialization.json.Json
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object NetworkModule {

    @Provides
    @Singleton
    fun provideKtorClient(): HttpClient = HttpClient(CIO) {
        install(HttpTimeout) {
            requestTimeoutMillis = 60_000
            connectTimeoutMillis = 20_000
            socketTimeoutMillis = 60_000
        }
        install(ContentNegotiation) {
            json(Json {
                ignoreUnknownKeys = true
                isLenient = true
                prettyPrint = false
            })
        }
        install(Logging) {
            level = LogLevel.INFO
        }
    }

    @Provides
    @Singleton
    fun provideSseClient(client: HttpClient): SseClient = SseClient(client)

    @Provides
    @Singleton
    fun provideBackendApi(client: HttpClient, prefs: PreferenceDataStore): BackendApiService = BackendApiServiceImpl(client, prefs)

    @Provides
    @Singleton
    fun provideWebHostApi(
        client: HttpClient,
        sseClient: SseClient,
        clientFactory: WebHostHttpClientFactory
    ): WebHostApiService = WebHostApiServiceImpl(client, sseClient, clientFactory)
}
