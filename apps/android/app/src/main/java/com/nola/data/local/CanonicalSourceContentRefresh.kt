package com.nola.data.local

import com.nola.domain.model.CustomDataSourceConfig
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

/** Owns a single Host read attempt, never configuration or default selection. */
internal class CanonicalSourceContentRefresh(
    private val client: CanonicalDataSourceClient,
    private val cacheDirectory: File
) {
    suspend fun execute(
        source: CustomDataSourceConfig,
        read: suspend () -> CustomDataSourceConfig
    ): CanonicalDataSource = withContext(Dispatchers.IO) {
        val pending = client.requestRefresh(source.id, requireNotNull(source.canonicalRevision),
            "Refresh data source", System.currentTimeMillis())
        val token = pending.readState.getValue("read_token").jsonPrimitive.content
        var file: File? = null
        try {
            val result = read()
            check(result.lastError == null) { requireNotNull(result.lastError) }
            require(result.id == source.id && result.location == source.location) { "Read result source mismatch" }
            val previous = requireNotNull(source.validationContract)
            val contract = requireNotNull(result.validationContract)
            val oldRecipe = requireNotNull(previous.readRecipe)
            val recipe = requireNotNull(contract.readRecipe)
            require(contract.sourceId == previous.sourceId && contract.connectorId == previous.connectorId &&
                contract.connectorVersion == previous.connectorVersion && contract.schemaFields == previous.schemaFields &&
                recipe.method == oldRecipe.method && recipe.operationId == oldRecipe.operationId &&
                recipe.configurationKeys == oldRecipe.configurationKeys &&
                Json.parseToJsonElement(recipe.outputSchemaJson) == Json.parseToJsonElement(oldRecipe.outputSchemaJson)) {
                "Source schema changed; validate the source again"
            }
            val bytes = result.cachedContent.toByteArray(Charsets.UTF_8)
            require(bytes.size <= 8 * 1024 * 1024) { "Data source exceeds the cache size limit" }
            check(cacheDirectory.isDirectory || cacheDirectory.mkdirs()) { "Cannot create data source cache directory" }
            val contentFile = File(cacheDirectory, "${UUID.randomUUID()}.txt")
            file = contentFile
            contentFile.writeBytes(bytes)
            val committed = client.recordRead(source.id, token, buildJsonObject {
                if (bytes.isEmpty()) {
                    put("status", "empty")
                } else {
                    put("status", "succeeded")
                    put("content_reference", buildJsonObject {
                        put("uri", contentFile.toURI().toString())
                        put("mime_type", "text/plain")
                        put("byte_count", bytes.size)
                        put("content_version", contentFile.nameWithoutExtension)
                    })
                    put("record_count", contract.recordCount)
                }
                put("observed_at_ms", contract.observedAt)
                put("schema_version", contract.connectorVersion)
            }, System.currentTimeMillis())
            if (bytes.isEmpty()) contentFile.delete()
            committed
        } catch (error: Exception) {
            val recovered = try {
                withContext(NonCancellable) {
                    val current = requireNotNull(client.get(source.id))
                    val reference = current.readState["content_reference"]
                        ?.takeUnless { it == JsonNull }?.jsonObject
                    val committedUri = reference?.get("uri")?.jsonPrimitive?.content
                    if (file != null && committedUri == file?.toURI()?.toString()) {
                        current
                    } else {
                        file?.delete()
                        val currentToken = current.readState["read_token"]
                            ?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
                        if (currentToken == token) client.recordRead(source.id, token, buildJsonObject {
                            put("status", "failed")
                            put("code", if (error is CancellationException) "host_read_cancelled" else "host_read_failed")
                            put("diagnostic", error.message ?: error.javaClass.simpleName)
                        }, System.currentTimeMillis()) else current
                    }
                }
            } catch (reconciliationError: Exception) {
                error.addSuppressed(reconciliationError)
                throw error
            }
            if (error is CancellationException) throw error
            recovered
        }
    }
}
