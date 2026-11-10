package com.nola.data.remote.api

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.dto.AuthResponse
import com.nola.data.remote.dto.DeviceRegisterRequest
import com.nola.data.remote.dto.DeviceRegisterResponse
import com.nola.data.remote.dto.ModelsListResponse
import com.nola.data.remote.dto.NotificationsListResponse
import com.nola.data.remote.dto.SourceSyncRequest
import com.nola.data.remote.dto.SourceSyncResponse
import com.nola.data.remote.dto.UnreadCountResponse
import com.nola.data.remote.dto.UserInfoResponse

interface BackendApiService {
    suspend fun authRegister(email: String, password: String, displayName: String? = null): AuthResponse
    suspend fun authLogin(email: String, password: String): AuthResponse
    suspend fun authMe(): UserInfoResponse
    suspend fun listModels(): ModelsListResponse
    suspend fun registerDevice(request: DeviceRegisterRequest): DeviceRegisterResponse
    suspend fun syncSources(request: SourceSyncRequest): SourceSyncResponse
    suspend fun getNotifications(limit: Int = 50, unreadOnly: Boolean = false): NotificationsListResponse
    suspend fun getUnreadCount(): UnreadCountResponse
    suspend fun markNotificationRead(id: String)
    suspend fun markAllNotificationsRead()
    suspend fun getAgentSettings(): AgentSettingsResponse
    suspend fun saveAgentSettings(settings: Map<String, Any>): AgentSettingsResponse
}
