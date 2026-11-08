package com.nola.data.remote.api

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.dto.DeviceRegisterRequest
import com.nola.data.remote.dto.DeviceRegisterResponse
import com.nola.data.remote.dto.NotificationsListResponse
import com.nola.data.remote.dto.SourceSyncRequest
import com.nola.data.remote.dto.SourceSyncResponse
import com.nola.data.remote.dto.UnreadCountResponse
import com.nola.data.remote.dto.UserInfoResponse
import com.nola.data.remote.dto.AuthResponse
import com.nola.data.remote.dto.ModelsListResponse
import com.nola.data.local.datastore.PreferenceDataStore
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.delete
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.http.ContentType
import io.ktor.http.contentType
import kotlinx.coroutines.flow.first
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class BackendApiServiceImpl @Inject constructor(
    private val client: HttpClient,
    private val prefs: PreferenceDataStore
) : BackendApiService {

    private suspend fun baseUrl(): String = prefs.backendBaseUrl.first().ifEmpty {
        "https://api.nola.ai"
    }

    private suspend fun token(): String = prefs.backendAccessToken.first()
    private suspend fun clientUuid(): String = prefs.getOrCreateClientUuid()

    private suspend fun authHeader(): String = "Bearer ${token()}"

    private suspend fun requireToken() {
        val t = token()
        if (t.isEmpty()) throw IllegalStateException("Not authenticated")
    }

    override suspend fun authRegister(email: String, password: String, displayName: String?): AuthResponse {
        return client.post("${baseUrl()}/v1/auth/register") {
            contentType(ContentType.Application.Json)
            setBody(
                mapOf(
                    "email" to email,
                    "password" to password,
                    "displayName" to displayName
                )
            )
        }.body()
    }

    override suspend fun authLogin(email: String, password: String): AuthResponse {
        return client.post("${baseUrl()}/v1/auth/login") {
            contentType(ContentType.Application.Json)
            setBody(mapOf("email" to email, "password" to password))
        }.body()
    }

    override suspend fun authMe(): UserInfoResponse {
        requireToken()
        return client.get("${baseUrl()}/v1/auth/me") {
            header("Authorization", authHeader())
            header("X-Nola-Access-Token", token())
        }.body()
    }

    override suspend fun listModels(): ModelsListResponse {
        return client.get("${baseUrl()}/v1/models") {
            header("X-Client-UUID", clientUuid())
            if (token().isNotEmpty()) {
                header("Authorization", authHeader())
            }
        }.body()
    }

    override suspend fun registerDevice(request: DeviceRegisterRequest): DeviceRegisterResponse {
        requireToken()
        return client.post("${baseUrl()}/v1/devices/register") {
            contentType(ContentType.Application.Json)
            header("Authorization", authHeader())
            header("X-Nola-Access-Token", token())
            setBody(request)
        }.body()
    }

    override suspend fun syncSources(request: SourceSyncRequest): SourceSyncResponse {
        requireToken()
        return client.post("${baseUrl()}/v1/sources/sync") {
            contentType(ContentType.Application.Json)
            header("Authorization", authHeader())
            header("X-Nola-Access-Token", token())
            setBody(request)
        }.body()
    }

    override suspend fun getNotifications(limit: Int, unreadOnly: Boolean): NotificationsListResponse {
        requireToken()
        val params = mutableListOf("limit=$limit")
        if (unreadOnly) params.add("unread_only=true")
        return client.get("${baseUrl()}/v1/notifications?${params.joinToString("&")}") {
            header("Authorization", authHeader())
        }.body()
    }

    override suspend fun getUnreadCount(): UnreadCountResponse {
        requireToken()
        return client.get("${baseUrl()}/v1/notifications/unread_count") {
            header("Authorization", authHeader())
        }.body()
    }

    override suspend fun markNotificationRead(id: String) {
        requireToken()
        client.post("${baseUrl()}/v1/notifications/$id/read") {
            header("Authorization", authHeader())
        }
    }

    override suspend fun markAllNotificationsRead() {
        requireToken()
        client.post("${baseUrl()}/v1/notifications/read_all") {
            header("Authorization", authHeader())
        }
    }

    override suspend fun getAgentSettings(): AgentSettingsResponse {
        requireToken()
        return client.get("${baseUrl()}/v1/agent/settings") {
            header("Authorization", authHeader())
        }.body()
    }

    override suspend fun saveAgentSettings(settings: Map<String, Any>): AgentSettingsResponse {
        requireToken()
        return client.post("${baseUrl()}/v1/agent/settings") {
            contentType(ContentType.Application.Json)
            header("Authorization", authHeader())
            setBody(settings)
        }.body()
    }
}
