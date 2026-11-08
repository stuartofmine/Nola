package com.nola.push

import com.nola.data.repository.AuthRepository
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class PushTokenManager @Inject constructor(
    private val authRepo: AuthRepository
) {
    suspend fun onNewToken(token: String) {
        try {
            authRepo.registerFcmToken(token)
        } catch (_: Exception) {
            // Will retry on next auth or app launch
        }
    }
}
