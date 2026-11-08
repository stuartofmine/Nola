package com.nola.data.repository

import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.remote.api.BackendApiService
import com.nola.data.remote.dto.AuthResponse
import com.nola.data.remote.dto.DeviceRegisterRequest
import kotlinx.coroutines.flow.first
import javax.inject.Inject
import javax.inject.Singleton

interface AuthRepository {
    suspend fun registerWithEmail(email: String, password: String, displayName: String? = null): AuthResponse
    suspend fun signInWithEmail(email: String, password: String): AuthResponse
    suspend fun linkBackend(token: String, userId: String)
    suspend fun unlinkBackend()
    suspend fun registerFcmToken(fcmToken: String)
    suspend fun isAuthenticated(): Boolean
    suspend fun getClientUuid(): String
}

@Singleton
class AuthRepositoryImpl @Inject constructor(
    private val api: BackendApiService,
    private val prefs: PreferenceDataStore
) : AuthRepository {

    override suspend fun registerWithEmail(email: String, password: String, displayName: String?): AuthResponse {
        val response = api.authRegister(email, password, displayName)
        persistAuth(response)
        return response
    }

    override suspend fun signInWithEmail(email: String, password: String): AuthResponse {
        val response = api.authLogin(email, password)
        persistAuth(response)
        return response
    }

    override suspend fun linkBackend(token: String, userId: String) {
        prefs.saveBackendConfig(
            baseUrl = prefs.backendBaseUrl.first().ifEmpty { "https://api.nola.ai" },
            token = token,
            userId = userId
        )
    }

    override suspend fun unlinkBackend() {
        prefs.clearBackendConfig()
    }

    override suspend fun registerFcmToken(fcmToken: String) {
        if (!isAuthenticated()) return
        try {
            api.registerDevice(DeviceRegisterRequest(deviceToken = fcmToken))
        } catch (_: Exception) { }
    }

    override suspend fun isAuthenticated(): Boolean {
        return prefs.backendAccessToken.first().isNotEmpty()
    }

    override suspend fun getClientUuid(): String = prefs.getOrCreateClientUuid()

    private suspend fun persistAuth(response: AuthResponse) {
        prefs.saveBackendConfig(
            baseUrl = prefs.backendBaseUrl.first().ifEmpty { "https://api.nola.ai" },
            token = response.accessToken,
            userId = response.userId
        )
    }
}
