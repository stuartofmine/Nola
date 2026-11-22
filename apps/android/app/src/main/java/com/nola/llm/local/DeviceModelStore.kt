package com.nola.llm.local

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.time.Instant
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

@Serializable
data class DeviceModelRecord(
    val id: String,
    val displayName: String,
    val fileName: String,
    val sourceUrl: String,
    val sha256: String,
    val byteSize: Long,
    val contextWindow: Int,
    val recommendedRamBytes: Long,
    val installedAt: String,
)

data class DeviceModelDownloadProgress(
    val downloadedBytes: Long,
    val totalBytes: Long?,
)

data class DeviceModelDownloadState(
    val downloadingModelId: String? = null,
    val downloadingDisplayName: String? = null,
    val progress: Float? = null,
    val completedModelId: String? = null,
    val completedDisplayName: String? = null,
    val error: String? = null,
)

@Singleton
class DeviceModelStore @Inject constructor(
    @ApplicationContext context: Context,
) {
    private val directory = File(context.filesDir, "models")
    private val manifest = File(directory, "manifest.json")
    private val json = Json { prettyPrint = true }
    private val downloadScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _downloadState = MutableStateFlow(DeviceModelDownloadState())
    val downloadState: StateFlow<DeviceModelDownloadState> = _downloadState.asStateFlow()
    private var activeDownloadJob: Job? = null

    fun startDownload(entry: DeviceModelCatalogEntry, source: LocalModelDownloadSource) {
        if (activeDownloadJob?.isActive == true) return
        _downloadState.value = DeviceModelDownloadState(
            downloadingModelId = entry.id,
            downloadingDisplayName = entry.displayName,
            progress = 0f,
        )
        activeDownloadJob = downloadScope.launch {
            try {
                install(
                    id = entry.id,
                    displayName = entry.displayName,
                    sourceUrl = entry.downloadUrl(source),
                    expectedSha256 = entry.sha256,
                    contextWindow = entry.contextWindow,
                    recommendedRamBytes = entry.recommendedRamBytes,
                ) { progress ->
                    val total = progress.totalBytes
                    _downloadState.update { state ->
                        state.copy(progress = total?.let { progress.downloadedBytes.toFloat() / it.toFloat() })
                    }
                }
                _downloadState.value = DeviceModelDownloadState(
                    completedModelId = entry.id,
                    completedDisplayName = entry.displayName,
                )
            } catch (cancelled: CancellationException) {
                _downloadState.value = DeviceModelDownloadState()
                throw cancelled
            } catch (error: Throwable) {
                _downloadState.value = DeviceModelDownloadState(
                    error = error.message.orEmpty(),
                )
            } finally {
                activeDownloadJob = null
            }
        }
    }

    fun cancelDownload() {
        activeDownloadJob?.cancel()
        activeDownloadJob = null
        _downloadState.update { it.copy(downloadingModelId = null, downloadingDisplayName = null, progress = null) }
    }

    fun consumeTerminalDownloadEvent() {
        _downloadState.update { it.copy(completedModelId = null, completedDisplayName = null, error = null) }
    }

    suspend fun installedModels(): List<DeviceModelRecord> = withContext(Dispatchers.IO) {
        readManifest().filter { record ->
            val file = modelFile(record)
            file.isFile && file.length() == record.byteSize
        }
    }

    suspend fun install(
        id: String,
        displayName: String,
        sourceUrl: String,
        expectedSha256: String,
        contextWindow: Int,
        recommendedRamBytes: Long,
        onProgress: (DeviceModelDownloadProgress) -> Unit = {},
    ): DeviceModelRecord = withContext(Dispatchers.IO) {
        require(id.matches(Regex("[A-Za-z0-9._-]+"))) { "Invalid model identifier" }
        require(expectedSha256.matches(Regex("[A-Fa-f0-9]{64}"))) { "A complete SHA-256 digest is required" }
        require(contextWindow > 0) { "Context window must be positive" }
        require(recommendedRamBytes > 0) { "Recommended RAM must be positive" }

        directory.mkdirs()
        val target = File(directory, "$id.gguf")
        val partial = File(directory, "$id.gguf.partial")

        var existingBytes = if (partial.isFile) partial.length() else 0L
        var retriedFromStart = false

        while (true) {
            val connection = (URL(sourceUrl).openConnection() as HttpURLConnection).apply {
                connectTimeout = 15_000
                readTimeout = 30_000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "Nola-Android")
                if (existingBytes > 0L) {
                    setRequestProperty("Range", "bytes=$existingBytes-")
                }
            }
            try {
                connection.connect()
                val statusCode = connection.responseCode
                if (statusCode == 416 && existingBytes > 0L && !retriedFromStart) {
                    partial.delete()
                    existingBytes = 0L
                    retriedFromStart = true
                    continue
                }
                require(statusCode in 200..299) {
                    "Model download failed with HTTP $statusCode"
                }
                val isPartialResume = statusCode == HttpURLConnection.HTTP_PARTIAL && existingBytes > 0L
                val resumeOffset = if (isPartialResume) existingBytes else 0L
                val contentLength = connection.contentLengthLong.takeIf { it > 0 }
                val total = contentLength?.let { if (isPartialResume) resumeOffset + it else it }
                if (resumeOffset > 0L) {
                    onProgress(DeviceModelDownloadProgress(resumeOffset, total))
                }
                connection.inputStream.use { input ->
                    java.io.FileOutputStream(partial, isPartialResume).buffered().use { output ->
                        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                        var downloaded = resumeOffset
                        while (true) {
                            currentCoroutineContext().ensureActive()
                            val count = input.read(buffer)
                            if (count < 0) break
                            output.write(buffer, 0, count)
                            downloaded += count
                            onProgress(DeviceModelDownloadProgress(downloaded, total))
                        }
                    }
                }
                break
            } finally {
                connection.disconnect()
            }
        }

        val actualSha256 = sha256(partial)
        if (!actualSha256.equals(expectedSha256, ignoreCase = true)) {
            partial.delete()
            throw IllegalArgumentException("Model checksum verification failed")
        }
        currentCoroutineContext().ensureActive()
        check(partial.renameTo(target)) { "Could not atomically install model" }

        val record = DeviceModelRecord(
            id = id,
            displayName = displayName,
            fileName = target.name,
            sourceUrl = sourceUrl,
            sha256 = actualSha256,
            byteSize = target.length(),
            contextWindow = contextWindow,
            recommendedRamBytes = recommendedRamBytes,
            installedAt = Instant.now().toString(),
        )
        writeManifest(readManifest().filterNot { it.id == id } + record)
        record
    }

    suspend fun delete(id: String, activeModelId: String?) = withContext(Dispatchers.IO) {
        require(id != activeModelId) { "Stop the model before deleting it" }
        val records = readManifest()
        val record = records.firstOrNull { it.id == id } ?: return@withContext
        check(modelFile(record).delete() || !modelFile(record).exists()) { "Could not delete model file" }
        writeManifest(records.filterNot { it.id == id })
    }

    fun modelPath(record: DeviceModelRecord): String = modelFile(record).absolutePath

    private fun modelFile(record: DeviceModelRecord): File = File(directory, record.fileName)

    private fun readManifest(): List<DeviceModelRecord> {
        if (!manifest.isFile) return emptyList()
        return json.decodeFromString(manifest.readText())
    }

    private fun writeManifest(records: List<DeviceModelRecord>) {
        directory.mkdirs()
        val temporary = File(directory, "manifest.json.partial")
        temporary.writeText(json.encodeToString(records))
        check(temporary.renameTo(manifest)) { "Could not atomically update model manifest" }
    }

    private suspend fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
            while (true) {
                currentCoroutineContext().ensureActive()
                val count = input.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }
}
