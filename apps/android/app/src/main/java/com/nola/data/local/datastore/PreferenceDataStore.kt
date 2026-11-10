package com.nola.data.local.datastore

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import com.nola.data.remote.api.normalizeWebHostBaseUrl
import com.nola.data.remote.api.normalizeCertificateSha256
import com.nola.data.remote.api.isAllowedWebHostTransport
import com.nola.data.local.security.SecureTokenStore
import com.nola.core.AndroidAgentPermissionMode
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import javax.inject.Inject
import javax.inject.Singleton

enum class AppearancePreference(val wireValue: String) { System("system"), Light("light"), Dark("dark") }

private val Context.preferenceDataStore: DataStore<Preferences> by preferencesDataStore(
    name = "nola_preferences"
)

@Singleton
class PreferenceDataStore @Inject constructor(
    @ApplicationContext private val context: Context,
    private val secureTokenStore: SecureTokenStore
) {
    companion object {
        private val hostConfigurationMutex = Mutex()
        private val KEY_APPEARANCE = stringPreferencesKey("appearance")
        private val KEY_HAS_COMPLETED_ONBOARDING = booleanPreferencesKey("has_completed_onboarding")
        private val KEY_ONBOARDING_RUNTIME_CHOICE = stringPreferencesKey("onboarding_runtime_choice")
        private val KEY_ONBOARDING_PLANNED_SOURCES = stringPreferencesKey("onboarding_planned_sources")
        private val KEY_BACKEND_BASE_URL = stringPreferencesKey("backend_base_url")
        private val KEY_BACKEND_USER_ID = stringPreferencesKey("backend_user_id")
        private val KEY_WEB_HOST_BASE_URL = stringPreferencesKey("web_host_base_url")
        private val KEY_WEB_HOST_ID = stringPreferencesKey("web_host_id")
        private val KEY_WEB_HOST_CERTIFICATE_SHA256 = stringPreferencesKey("web_host_certificate_sha256")
        private val KEY_DEFAULT_MODEL_SELECTION = stringPreferencesKey("default_model_selection")
        private val KEY_CLIENT_UUID = stringPreferencesKey("client_uuid")
        private val KEY_AGENT_MODE_ENABLED = booleanPreferencesKey("agent_mode_enabled")
        private val KEY_AGENT_PERMISSION_PROFILE = stringPreferencesKey("agent_permission_profile")
        private val KEY_HYBRID_DEVICE_TOOLS_ENABLED = booleanPreferencesKey("hybrid_device_tools_enabled")
        private val KEY_HYBRID_DEVICE_CAPABILITIES = stringSetPreferencesKey("hybrid_device_capabilities")
    }

    val appearance: Flow<AppearancePreference> = context.preferenceDataStore.data.map { preferences ->
        AppearancePreference.entries.firstOrNull { it.wireValue == preferences[KEY_APPEARANCE] } ?: AppearancePreference.System
    }

    suspend fun setAppearance(value: AppearancePreference) {
        context.preferenceDataStore.edit { it[KEY_APPEARANCE] = value.wireValue }
    }

    suspend fun saveDefaultModelSelection(model: com.nola.domain.model.ModelInfo) {
        val hostId = webHostIdentity.first()
        val hostUrl = webHostBaseUrl.first()
        val hostPin = webHostCertificateSha256.first()
        val value = kotlinx.serialization.json.buildJsonObject {
            put("source", kotlinx.serialization.json.JsonPrimitive(model.source.name))
            put("selection_id", kotlinx.serialization.json.JsonPrimitive(model.selectionId))
            if (model.source == com.nola.domain.model.ModelInfo.ModelSource.WebHost) {
                put("host_id", kotlinx.serialization.json.JsonPrimitive(hostId))
                put("url", kotlinx.serialization.json.JsonPrimitive(hostUrl))
                put("pin", kotlinx.serialization.json.JsonPrimitive(hostPin))
            }
        }
        context.preferenceDataStore.edit { it[KEY_DEFAULT_MODEL_SELECTION] = value.toString() }
    }

    suspend fun defaultModelSelection(models: List<com.nola.domain.model.ModelInfo>): com.nola.domain.model.ModelInfo? {
        val raw = context.preferenceDataStore.data.first()[KEY_DEFAULT_MODEL_SELECTION] ?: return null
        val value = kotlinx.serialization.json.Json.parseToJsonElement(raw) as? kotlinx.serialization.json.JsonObject ?: return null
        fun text(key: String) = (value[key] as? kotlinx.serialization.json.JsonPrimitive)?.content
        val model = models.singleOrNull { it.source.name == text("source") && it.selectionId == text("selection_id") } ?: return null
        if (model.source == com.nola.domain.model.ModelInfo.ModelSource.WebHost &&
            (text("host_id").isNullOrBlank() || text("host_id") != webHostIdentity.first() ||
                text("url") != webHostBaseUrl.first() || text("pin") != webHostCertificateSha256.first())) return null
        return model
    }

    val hasCompletedOnboarding: Flow<Boolean> = context.preferenceDataStore.data.map {
        it[KEY_HAS_COMPLETED_ONBOARDING] ?: false
    }

    val onboardingRuntimeChoice: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_ONBOARDING_RUNTIME_CHOICE] ?: "automatic"
    }

    val onboardingPlannedSources: Flow<Set<String>> = context.preferenceDataStore.data.map {
        it[KEY_ONBOARDING_PLANNED_SOURCES]
            ?.split(',')
            ?.filter(String::isNotBlank)
            ?.toSet()
            ?: setOf("documents")
    }

    val backendBaseUrl: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_BACKEND_BASE_URL] ?: ""
    }

    val backendAccessToken: Flow<String> = context.preferenceDataStore.data.map {
        secureTokenStore.readBackendAccessToken() ?: ""
    }

    val backendUserId: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_BACKEND_USER_ID] ?: ""
    }

    val webHostBaseUrl: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_WEB_HOST_BASE_URL] ?: ""
    }

    val webHostToken: Flow<String> = context.preferenceDataStore.data.map {
        secureTokenStore.readWebHostToken() ?: ""
    }

    val webHostIdentity: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_WEB_HOST_ID] ?: ""
    }

    val webHostCertificateSha256: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_WEB_HOST_CERTIFICATE_SHA256] ?: ""
    }

    val clientUuid: Flow<String> = context.preferenceDataStore.data.map {
        it[KEY_CLIENT_UUID] ?: ""
    }

    val isAgentModeEnabled: Flow<Boolean> = context.preferenceDataStore.data.map {
        it[KEY_AGENT_MODE_ENABLED] ?: false
    }

    val agentPermissionMode: Flow<AndroidAgentPermissionMode> = context.preferenceDataStore.data.map {
        AndroidAgentPermissionMode.fromWireValue(it[KEY_AGENT_PERMISSION_PROFILE])
    }

    val hybridDeviceToolsEnabled: Flow<Boolean> = context.preferenceDataStore.data.map {
        it[KEY_HYBRID_DEVICE_TOOLS_ENABLED] ?: false
    }

    val hybridDeviceCapabilities: Flow<Set<String>> = context.preferenceDataStore.data.map {
        it[KEY_HYBRID_DEVICE_CAPABILITIES] ?: emptySet()
    }

    suspend fun setOnboardingCompleted() {
        context.preferenceDataStore.edit { it[KEY_HAS_COMPLETED_ONBOARDING] = true }
    }

    suspend fun completeOnboarding(runtimeChoice: String, selectedSources: Set<String>) {
        context.preferenceDataStore.edit {
            it[KEY_ONBOARDING_RUNTIME_CHOICE] = runtimeChoice
            it[KEY_ONBOARDING_PLANNED_SOURCES] = selectedSources.sorted().joinToString(",")
            it[KEY_HAS_COMPLETED_ONBOARDING] = true
        }
    }

    suspend fun saveBackendConfig(baseUrl: String, token: String, userId: String) {
        require(token.isNotBlank()) { "Backend access token is required" }
        check(secureTokenStore.saveBackendAccessToken(token.trim())) {
            "Unable to store backend access token in Android Keystore"
        }
        context.preferenceDataStore.edit {
            it[KEY_BACKEND_BASE_URL] = baseUrl
            it[KEY_BACKEND_USER_ID] = userId
        }
    }

    suspend fun clearBackendConfig() {
        secureTokenStore.deleteBackendAccessToken()
        context.preferenceDataStore.edit {
            it.remove(KEY_BACKEND_USER_ID)
        }
    }

    suspend fun saveWebHostConfig(baseUrl: String, token: String, certificateSha256: String = "", verifiedHostId: String? = null) {
        hostConfigurationMutex.withLock {
            val normalized = normalizeWebHostBaseUrl(baseUrl)
            val normalizedToken = token.trim()
            val normalizedCertificate = normalizeCertificateSha256(certificateSha256)
                ?: throw IllegalArgumentException("Invalid SHA-256 certificate fingerprint")
            require(normalizedToken.isNotEmpty()) { "Web Host token is required" }
            require(isAllowedWebHostTransport(normalized)) {
                "Public Web Host addresses require HTTPS"
            }
            require(normalizedCertificate.isEmpty() || normalized.startsWith("https://", ignoreCase = true)) {
                "Certificate pinning requires HTTPS"
            }
            require(verifiedHostId == null || (verifiedHostId.length in 16..128 && verifiedHostId.all { it.isLetterOrDigit() || it == '-' || it == '_' })) {
                "Invalid verified Web Host identity"
            }
            var previousToken: String? = null
            var tokenWritten = false
            try { context.preferenceDataStore.edit {
                if (verifiedHostId != null && it[KEY_WEB_HOST_BASE_URL] == normalized) {
                    val existing = it[KEY_WEB_HOST_ID].orEmpty()
                    check(existing.isEmpty() || existing == verifiedHostId) { "Web Host identity changed before saving" }
                }
                previousToken = secureTokenStore.readWebHostToken()
                check(secureTokenStore.saveWebHostToken(normalizedToken)) { "Unable to store Web Host token in Android Keystore" }
                tokenWritten = true
                if (it[KEY_WEB_HOST_BASE_URL] != normalized) {
                    it.remove(KEY_WEB_HOST_ID)
                    it.remove(KEY_WEB_HOST_CERTIFICATE_SHA256)
                    it[KEY_HYBRID_DEVICE_TOOLS_ENABLED] = false
                    it.remove(KEY_HYBRID_DEVICE_CAPABILITIES)
                }
                it[KEY_WEB_HOST_BASE_URL] = normalized
                if (verifiedHostId != null) it[KEY_WEB_HOST_ID] = verifiedHostId
                if (normalizedCertificate.isEmpty()) {
                    it.remove(KEY_WEB_HOST_CERTIFICATE_SHA256)
                } else {
                    it[KEY_WEB_HOST_CERTIFICATE_SHA256] = normalizedCertificate
                }
            } } catch (error: Exception) {
                if (tokenWritten) {
                    val previous = previousToken
                    if (previous == null) secureTokenStore.deleteWebHostToken()
                    else check(secureTokenStore.saveWebHostToken(previous)) { "Unable to restore the previous Web Host credential" }
                }
                throw error
            }
        }
    }

    suspend fun trustWebHostIdentity(baseUrl: String, hostId: String): Boolean {
        val normalizedUrl = normalizeWebHostBaseUrl(baseUrl)
        val identity = hostId.trim()
        if (identity.length !in 16..128 || identity.any { !it.isLetterOrDigit() && it != '-' && it != '_' }) {
            return false
        }
        var trusted = false
        context.preferenceDataStore.edit { preferences ->
            if (preferences[KEY_WEB_HOST_BASE_URL] != normalizedUrl) return@edit
            val existing = preferences[KEY_WEB_HOST_ID].orEmpty()
            trusted = existing.isEmpty() || existing == identity
            if (trusted && existing.isEmpty()) preferences[KEY_WEB_HOST_ID] = identity
        }
        return trusted
    }

    suspend fun forgetWebHostIdentity() {
        context.preferenceDataStore.edit {
            it.remove(KEY_WEB_HOST_ID)
            it.remove(KEY_WEB_HOST_CERTIFICATE_SHA256)
            it[KEY_HYBRID_DEVICE_TOOLS_ENABLED] = false
            it.remove(KEY_HYBRID_DEVICE_CAPABILITIES)
        }
    }

    suspend fun clearWebHostConfig() {
        hostConfigurationMutex.withLock {
        secureTokenStore.deleteWebHostToken()
        context.preferenceDataStore.edit {
            it.remove(KEY_WEB_HOST_BASE_URL)
            it.remove(KEY_WEB_HOST_ID)
            it.remove(KEY_WEB_HOST_CERTIFICATE_SHA256)
        }
        }
    }

    suspend fun setAgentModeEnabled(enabled: Boolean) {
        context.preferenceDataStore.edit { it[KEY_AGENT_MODE_ENABLED] = enabled }
    }

    suspend fun setAgentPermissionMode(mode: AndroidAgentPermissionMode) {
        context.preferenceDataStore.edit { it[KEY_AGENT_PERMISSION_PROFILE] = mode.wireValue }
    }

    suspend fun setHybridDeviceToolsEnabled(enabled: Boolean) {
        context.preferenceDataStore.edit { it[KEY_HYBRID_DEVICE_TOOLS_ENABLED] = enabled }
    }

    suspend fun setHybridDeviceCapability(capability: String, enabled: Boolean) {
        context.preferenceDataStore.edit { preferences ->
            val capabilities = preferences[KEY_HYBRID_DEVICE_CAPABILITIES]?.toMutableSet() ?: mutableSetOf()
            if (enabled) capabilities += capability else capabilities -= capability
            preferences[KEY_HYBRID_DEVICE_CAPABILITIES] = capabilities
        }
    }

    suspend fun getOrCreateClientUuid(): String {
        val existing = clientUuid.first()
        if (existing.isNotBlank()) return existing

        val generated = java.util.UUID.randomUUID().toString()
        context.preferenceDataStore.edit { prefs ->
            prefs[KEY_CLIENT_UUID] = prefs[KEY_CLIENT_UUID] ?: generated
        }
        return clientUuid.first().ifBlank { generated }
    }

    suspend fun clearAll() {
        secureTokenStore.clearAll()
        context.preferenceDataStore.edit { it.clear() }
    }
}
