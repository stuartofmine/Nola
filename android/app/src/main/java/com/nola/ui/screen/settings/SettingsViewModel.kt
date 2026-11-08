package com.nola.ui.screen.settings

import android.content.Context
import com.nola.data.local.AndroidApplicationRuntime
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.db.NolaDatabase
import com.nola.work.ScheduledActionScheduler
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
import java.text.SimpleDateFormat
import java.net.URI
import java.net.URLDecoder
import java.util.Date
import java.util.Locale
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
    val localDataSizeBytes: Long? = null
)

@HiltViewModel
class SettingsViewModel @Inject constructor(
    @ApplicationContext private val context: Context,
    private val prefs: PreferenceDataStore,
    private val memoryProfileDataStore: MemoryProfileDataStore,
    private val modelParametersStore: ModelParametersStore,
    private val database: NolaDatabase,
    private val scheduledActionScheduler: ScheduledActionScheduler
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
                _uiState.value = loaded
            }.onFailure {
                _uiState.update { it.copy(isLoading = false, loadFailed = true) }
            }
        }
    }

    fun localDataSizeBytes(): Long {
        val dbFile = context.getDatabasePath("nola.db")
        val canonicalDatabaseFile = AndroidApplicationRuntime.defaultDatabaseFile(context)
        val prefsDir = File(context.filesDir.parent, "shared_prefs")
        val datastoreDir = File(context.filesDir, "datastore")

        var bytes = 0L
        if (dbFile.exists()) bytes += dbFile.length()
        if (canonicalDatabaseFile.exists()) bytes += canonicalDatabaseFile.length()
        prefsDir.listFiles()?.forEach { bytes += it.length() }
        datastoreDir.walkTopDown().filter { it.isFile }.forEach { bytes += it.length() }
        return bytes
    }

    suspend fun createExportArchive(): File = withContext(Dispatchers.IO) {
        prepareDatabaseForExport()

        val exportDir = File(context.cacheDir, "export").apply { mkdirs() }
        exportDir.listFiles()
            ?.filter { it.name.startsWith("nola_backup_") && it.extension == "zip" }
            ?.forEach { it.delete() }

        val exportFile = File(
            exportDir,
            "nola_backup_${SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())}.zip"
        )

        ZipOutputStream(FileOutputStream(exportFile)).use { zip ->
            addFileIfExists(zip, context.getDatabasePath("nola.db"), "database/nola.db")
            addFileIfExists(
                zip,
                AndroidApplicationRuntime.defaultDatabaseFile(context),
                "database/nola-product.db",
            )
            val json = Json { prettyPrint = true; encodeDefaults = true }
            var structuredCredentialRedactions = 0
            val memoryProfile = memoryProfileDataStore.getSnapshot()
            val exportableMemoryProfile = memoryProfile.copy(
                mcpServers = memoryProfile.mcpServers.map { server ->
                    val sanitized = sanitizeMcpUrlForExport(server.url)
                    structuredCredentialRedactions += sanitized.redactionCount
                    server.copy(url = sanitized.url)
                }
            )
            addTextEntry(zip, "profile/memory.json", json.encodeToString(exportableMemoryProfile))
            addTextEntry(zip, "settings/model_parameters.json", json.encodeToString(modelParametersStore.exportSnapshot()))
            val safeSettings = buildJsonObject {
                put("auto_compact_percent", memoryProfileDataStore.autoCompactTriggerPercent.first())
                put("agent_mode_enabled", prefs.isAgentModeEnabled.first())
                put("permission_profile", prefs.agentPermissionMode.first().wireValue)
                put("hybrid_device_tools_enabled", prefs.hybridDeviceToolsEnabled.first())
                put("hybrid_device_capabilities", JsonArray(prefs.hybridDeviceCapabilities.first().sorted().map(::JsonPrimitive)))
                put("web_host_url", prefs.webHostBaseUrl.first())
                put("web_host_identity", prefs.webHostIdentity.first())
                put("web_host_certificate_sha256", prefs.webHostCertificateSha256.first())
                put("backend_url", prefs.backendBaseUrl.first())
                put("backend_user_id", prefs.backendUserId.first())
            }
            addTextEntry(zip, "settings/preferences.json", json.encodeToString(safeSettings))
            addTextEntry(
                zip,
                "redactions.json",
                json.encodeToString(buildJsonObject {
                    put("structured_mcp_url_credentials", structuredCredentialRedactions)
                    put("free_text_scanned", false)
                })
            )

            val metadata = """
                Nola Android user-initiated export
                created_at=${Date()}
                format_version=2
                includes=conversation database, memory profile, model parameters, safe preferences
                excludes=stored service access tokens, model API keys, Keystore material, secure preferences, client UUID, WAL/SHM files, structured MCP URL credentials
                redactions=See redactions.json. Free-text content is preserved and is not scanned or rewritten.
                warning=This archive can contain private conversations, memories, documents, custom prompts, and tool history. Store it securely.
            """.trimIndent() + "\n"
            addTextEntry(zip, "README.txt", metadata)
        }

        exportFile
    }

    suspend fun clearAllData() {
        scheduledActionScheduler.cancelAll()
        withContext(Dispatchers.IO) {
            database.clearAllTables()
        }
        prefs.clearAll()
        memoryProfileDataStore.clearAll()
        modelParametersStore.clearAll()
        val clientUuid = prefs.getOrCreateClientUuid()
        _uiState.update {
            SettingsUiState(
                isLoading = false,
                clientUuid = clientUuid,
                localDataSizeBytes = 0
            )
        }
    }

    private fun prepareDatabaseForExport() {
        runCatching {
            val db = database.openHelper.writableDatabase
            db.query("PRAGMA wal_checkpoint(TRUNCATE)").close()
            db.execSQL("VACUUM")
        }
    }

    private fun addFileIfExists(zip: ZipOutputStream, file: File, entryName: String) {
        if (!file.exists() || !file.isFile) return
        zip.putNextEntry(ZipEntry(entryName))
        file.inputStream().use { input -> input.copyTo(zip) }
        zip.closeEntry()
    }

    private fun addTextEntry(zip: ZipOutputStream, entryName: String, content: String) {
        zip.putNextEntry(ZipEntry(entryName))
        zip.write(content.toByteArray(Charsets.UTF_8))
        zip.closeEntry()
    }
}

internal data class SanitizedMcpExportUrl(
    val url: String,
    val redactionCount: Int
)

internal fun sanitizeMcpUrlForExport(raw: String): SanitizedMcpExportUrl {
    val uri = runCatching { URI(raw.trim()) }.getOrNull()
        ?: return SanitizedMcpExportUrl(raw, 0)
    if (uri.scheme.isNullOrBlank() || uri.host.isNullOrBlank()) {
        return SanitizedMcpExportUrl(raw, 0)
    }
    var redactions = if (uri.userInfo.isNullOrEmpty()) 0 else 1
    val sensitiveKeys = setOf(
        "token", "accesstoken", "apikey", "key", "secret",
        "password", "authorization", "auth", "bearertoken"
    )
    val query = uri.rawQuery?.split('&')?.joinToString("&") { item ->
        val rawKey = item.substringBefore('=')
        val decodedKey = runCatching { URLDecoder.decode(rawKey, "UTF-8") }.getOrDefault(rawKey)
        val normalizedKey = decodedKey.lowercase().filter(Char::isLetterOrDigit)
        if (normalizedKey in sensitiveKeys) {
            redactions += 1
            "$rawKey=REDACTED"
        } else {
            item
        }
    }
    if (!uri.rawFragment.isNullOrEmpty()) redactions += 1
    val sanitized = runCatching {
        URI(uri.scheme, null, uri.host, uri.port, uri.path, query, null).toString()
    }.getOrDefault(raw)
    return SanitizedMcpExportUrl(sanitized, redactions)
}
