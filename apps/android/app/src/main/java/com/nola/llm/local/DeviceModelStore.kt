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
import kotlinx.coroutines.Dispatchers
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

@Singleton
class DeviceModelStore @Inject constructor(
    @ApplicationContext context: Context,
) {
    private val directory = File(context.filesDir, "models")
    private val manifest = File(directory, "manifest.json")
    private val json = Json { prettyPrint = true }

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
        partial.delete()

        try {
            val connection = (URL(sourceUrl).openConnection() as HttpURLConnection).apply {
                connectTimeout = 15_000
                readTimeout = 30_000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "Nola-Android")
            }
            connection.connect()
            require(connection.responseCode in 200..299) {
                "Model download failed with HTTP ${connection.responseCode}"
            }
            val total = connection.contentLengthLong.takeIf { it > 0 }
            connection.inputStream.use { input ->
                partial.outputStream().buffered().use { output ->
                    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                    var downloaded = 0L
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        output.write(buffer, 0, count)
                        downloaded += count
                        onProgress(DeviceModelDownloadProgress(downloaded, total))
                    }
                }
            }
            connection.disconnect()

            val actualSha256 = sha256(partial)
            require(actualSha256.equals(expectedSha256, ignoreCase = true)) {
                "Model checksum verification failed"
            }
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
        } catch (error: Throwable) {
            partial.delete()
            throw error
        }
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

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }
}
