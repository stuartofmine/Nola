package com.nola.ui.screen.settings

import android.app.ActivityManager
import android.content.Context
import com.nola.data.local.AndroidApplicationRuntime
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.domain.model.ChatMemoryProfile
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream
import java.util.Date
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import javax.inject.Inject

data class SettingsUiState(
    val isLoading: Boolean = true,
    val loadFailed: Boolean = false,
    val backendLinked: Boolean = false,
    val responseStyle: String = "casual",
    val agentModeEnabled: Boolean = false,
    val autoCompactPercent: Int = 80,
    val apiEndpoint: String = "",
    val apiModel: String = "",
    val webHostUrl: String = "",
    val webHostConfigured: Boolean = false,
    val webHostIdentity: String = "",
    val webHostCertificateSha256: String = "",
    val clientUuid: String = "",
    val localDataSizeBytes: Long? = null,
    val isExportingData: Boolean = false,
    val exportedArchiveFile: File? = null,
    val exportErrorMessage: String? = null,
)

@HiltViewModel
class SettingsViewModel @Inject constructor(
    @ApplicationContext private val context: Context,
    private val prefs: PreferenceDataStore,
    private val memoryProfileDataStore: MemoryProfileDataStore,
    private val modelParametersStore: ModelParametersStore,
    private val runtime: AndroidApplicationRuntime
) : ViewModel() {

    val appearance = prefs.appearance

    fun setAppearance(value: com.nola.data.local.datastore.AppearancePreference) {
        viewModelScope.launch { prefs.setAppearance(value) }
    }

    private val _uiState = MutableStateFlow(SettingsUiState())
    val uiState: StateFlow<SettingsUiState> = _uiState.asStateFlow()

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            _uiState.update { it.copy(isLoading = true, loadFailed = false) }
            runCatching {
                val apiModel = memoryProfileDataStore.agentLlmModel.first()
                val apiEndpoint = memoryProfileDataStore.agentLlmEndpoint.first()
                val webHostUrl = prefs.webHostBaseUrl.first()
                val webHostToken = prefs.webHostToken.first()
                val webHostIdentity = prefs.webHostIdentity.first()
                val webHostCertificateSha256 = prefs.webHostCertificateSha256.first()
                val backendLinked = prefs.backendAccessToken.first().isNotBlank()
                val responseStyle = memoryProfileDataStore.responseStyle.first()
                val agentModeEnabled = prefs.isAgentModeEnabled.first()
                val autoCompactPercent = memoryProfileDataStore.autoCompactTriggerPercent.first()
                val clientUuid = prefs.getOrCreateClientUuid()
                val dataSize = withContext(Dispatchers.IO) { localDataSizeBytes() }
                SettingsUiState(
                    isLoading = false,
                    backendLinked = backendLinked,
                    responseStyle = responseStyle,
                    agentModeEnabled = agentModeEnabled,
                    autoCompactPercent = autoCompactPercent,
                    apiEndpoint = if (apiModel != "web-host") apiEndpoint else "",
                    apiModel = if (apiModel != "web-host") apiModel else "",
                    webHostUrl = webHostUrl,
                    webHostConfigured = webHostUrl.isNotBlank() && webHostToken.isNotBlank(),
                    webHostIdentity = webHostIdentity,
                    webHostCertificateSha256 = webHostCertificateSha256,
                    clientUuid = clientUuid,
                    localDataSizeBytes = dataSize
                )
            }.onSuccess { loaded ->
                _uiState.update { current ->
                    loaded.copy(
                        isExportingData = current.isExportingData,
                        exportedArchiveFile = current.exportedArchiveFile,
                        exportErrorMessage = current.exportErrorMessage,
                    )
                }
            }.onFailure { error ->
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update { it.copy(isLoading = false, loadFailed = true) }
            }
        }
    }

    fun localDataSizeBytes(): Long {
        val privateRoots = setOf(context.dataDir, context.createDeviceProtectedStorageContext().dataDir)
        return privateRoots.sumOf { root ->
            root.walkTopDown().onEnter { !java.nio.file.Files.isSymbolicLink(it.toPath()) }
                .filter { it.isFile && !java.nio.file.Files.isSymbolicLink(it.toPath()) }.sumOf { it.length() }
        }
    }

    suspend fun createExportArchive(): File = withContext(Dispatchers.IO) {
        val exportDir = File(context.cacheDir, "export").apply { mkdirs() }
        val now = System.currentTimeMillis()
        exportDir.listFiles()?.filter {
            it.isFile && it.extension == "zip" && now - it.lastModified() > 24 * 60 * 60 * 1000L
        }?.forEach { it.delete() }
        val exportFile = File.createTempFile("nola_export_", ".zip", exportDir)
        try {
            ZipOutputStream(FileOutputStream(exportFile).buffered()).use { zip ->
                writeConversationDocuments(zip, runtime)
                val json = Json { prettyPrint = true; encodeDefaults = true }
                addTextEntry(zip, "manifest.json", """{"format":"nola.android.personal-export","version":2,"transcripts":"markdown"}""")
                val profile = memoryProfileDataStore.getSnapshot()
                addTextEntry(zip, "profile/memory.json", json.encodeToString(exportableProfile(profile)))
                addTextEntry(zip, "settings/model_parameters.json", json.encodeToString(modelParametersStore.exportSnapshot()))
                val preferences = buildJsonObject {
                    put("auto_compact_percent", memoryProfileDataStore.autoCompactTriggerPercent.first())
                    put("agent_mode_enabled", prefs.isAgentModeEnabled.first())
                    put("permission_profile", prefs.agentPermissionMode.first().wireValue)
                    put("hybrid_device_tools_enabled", prefs.hybridDeviceToolsEnabled.first())
                    put("hybrid_device_capabilities", JsonArray(prefs.hybridDeviceCapabilities.first().sorted().map(::JsonPrimitive)))
                }
                addTextEntry(zip, "settings/preferences.json", json.encodeToString(preferences))
                addTextEntry(zip, "README.txt", """
                    Nola Android personal export
                    created_at=${Date()}
                    format=nola.android.personal-export
                    format_version=2
                    conversations/*.md contains user and assistant text. Markdown can be opened in a text editor or added to the document library.
                    profile/memory.json contains memories, preferences, user documents and skills.
                    settings/ contains model parameters and local agent preferences.
                    Connection details, tool inputs and outputs, credentials and raw databases are excluded.
                    User text is preserved as entered and may include private information.
                """.trimIndent() + "\n")
            }
            exportFile
        } catch (error: Throwable) {
            exportFile.delete()
            throw error
        }
    }

    fun startExportArchive() {
        if (_uiState.value.isExportingData) return
        _uiState.update { it.copy(isExportingData = true, exportedArchiveFile = null, exportErrorMessage = null) }
        viewModelScope.launch {
            try {
                val exportFile = createExportArchive()
                _uiState.update {
                    it.copy(
                        isExportingData = false,
                        exportedArchiveFile = exportFile,
                        exportErrorMessage = null,
                    )
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                _uiState.update { it.copy(isExportingData = false) }
                throw cancelled
            } catch (e: Exception) {
                _uiState.update {
                    it.copy(
                        isExportingData = false,
                        exportErrorMessage = e.message ?: "",
                    )
                }
            }
        }
    }

    fun consumeExportResult() {
        _uiState.update { it.copy(exportedArchiveFile = null, exportErrorMessage = null) }
    }

    fun clearAllData(): Boolean {
        // Android stops the process and clears all private stores together,
        // including the live Core database, credentials and WorkManager state.
        return context.getSystemService(ActivityManager::class.java).clearApplicationUserData()
    }

    private fun addTextEntry(zip: ZipOutputStream, entryName: String, content: String) {
        zip.putNextEntry(ZipEntry(entryName))
        zip.write(content.toByteArray(Charsets.UTF_8))
        zip.closeEntry()
    }
}

/** Export text the user authored, without serializing connection configuration or authority records. */
internal fun exportableProfile(profile: ChatMemoryProfile): ChatMemoryProfile = ChatMemoryProfile(
    longTermMemory = profile.longTermMemory,
    userPreferences = profile.userPreferences,
    uploadedSkills = profile.uploadedSkills.map { it.copy(accessGrants = emptyList(), accessOwnerId = null) },
    uploadedDocuments = profile.uploadedDocuments.map { it.copy(accessGrants = emptyList(), accessOwnerId = null) },
)
