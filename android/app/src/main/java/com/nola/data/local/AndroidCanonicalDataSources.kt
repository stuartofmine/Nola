package com.nola.data.local

import com.nola.domain.model.*
import kotlinx.serialization.json.*
import java.io.File
import java.time.Instant
import java.util.UUID

/** Maps UI values to Application commands; cache bytes remain Host-owned. */
internal class AndroidCanonicalDataSources(
    private val client: CanonicalDataSourceClient,
    private val cacheDirectory: File,
) {
    fun list(): List<CustomDataSourceConfig> = client.list().map(::project)

    suspend fun refresh(source: CustomDataSourceConfig, read: suspend () -> CustomDataSourceConfig): CustomDataSourceConfig =
        project(CanonicalSourceContentRefresh(client, cacheDirectory).execute(source, read))

    fun delete(id: String) {
        val source = requireNotNull(client.get(id)) { "Data source no longer exists" }
        client.delete(id, source.revision, System.currentTimeMillis())
    }

    fun setEnabled(id: String, enabled: Boolean) {
        val source = requireNotNull(client.get(id)) { "Data source no longer exists" }
        client.setEnabled(id, source.revision, enabled, System.currentTimeMillis())
    }

    fun save(source: CustomDataSourceConfig) {
        val existing = client.get(source.id)
        if (existing != null) {
            require(source.canonicalRevision == existing.revision) { "Data source changed; reload before saving" }
        }
        val now = System.currentTimeMillis()
        if (source.lastError != null && existing != null) {
            val pending = client.requestRefresh(source.id, existing.revision, "Refresh source", now)
            client.recordRead(source.id, pending.readState.string("read_token"), buildJsonObject {
                put("status", "failed"); put("code", "host_read_failed"); put("diagnostic", source.lastError)
            }, now)
            return
        }
        val contract = requireNotNull(source.validationContract) { "Validate the data source before saving" }
        val bytes = source.cachedContent.toByteArray(Charsets.UTF_8)
        require(bytes.size <= MAX_CACHE_BYTES) { "Data source exceeds the cache size limit" }
        val draft = CanonicalDataSourceDraft(
            id = source.id, name = source.name, kind = when (source.kind) {
                UnifiedDataSourceKind.File, UnifiedDataSourceKind.UploadedDocument -> "local_file"
                UnifiedDataSourceKind.Folder -> "directory"
                UnifiedDataSourceKind.Database -> "database"
                UnifiedDataSourceKind.Mcp -> "mcp"
                UnifiedDataSourceKind.Health, UnifiedDataSourceKind.Calendar, UnifiedDataSourceKind.Reminder -> "platform"
                UnifiedDataSourceKind.Custom -> "custom"
                else -> "http"
            },
            configuration = CanonicalDataSourceConfiguration(
                connectorId = contract.connectorId,
                locationKind = when (contract.readRecipe?.method) {
                    DataSourceReadMethod.LocalFileRead, DataSourceReadMethod.DirectoryList -> "local_path"
                    DataSourceReadMethod.DatabaseQuery -> "database_reference"
                    DataSourceReadMethod.PlatformApi -> "platform_resource"
                    DataSourceReadMethod.McpTool -> "mcp_capability"
                    DataSourceReadMethod.HttpGet -> "http_url"
                    else -> "opaque_reference"
                }, location = source.location,
            ),
            accessPolicy = source.accessPolicy.name.replaceFirstChar { it.lowercase() },
            // Successful HTTP reads do not establish native system permissions.
            authorization = CanonicalDataSourceAuthorization(
                state = if (contract.readRecipe?.method == DataSourceReadMethod.HttpGet) "not_required"
                    else existing?.authorization?.state ?: "not_determined",
                capability = existing?.authorization?.capability, observedAtMs = now,
            ),
            freshnessPolicy = CanonicalDataSourceFreshness(source.maximumAgeMinutes.toLong(), false),
            capabilities = listOf("read", "context"),
            privacy = if (source.allowCloudContext) "allow-cloud-context" else "local-only",
            createdAtMs = existing?.createdAtMs ?: source.createdAt, updatedAtMs = now,
        )
        val staged = if (existing == null) client.create(draft) else client.update(draft, existing.revision)
        val pending = client.requestValidation(staged.id, staged.revision, now)
        val validated = client.recordValidation(staged.id, requireNotNull(pending.validation.validationToken),
            buildJsonObject { put("status", "succeeded"); put("contract", contract.toCanonicalContract()) }, now)
        check(cacheDirectory.isDirectory || cacheDirectory.mkdirs()) { "Cannot create data source cache directory" }
        val file = File(cacheDirectory, "${UUID.randomUUID()}.txt")
        try { file.writeBytes(bytes) } catch (error: Exception) { file.delete(); throw error }
        var readToken: String? = null
        val stored = try {
            val reading = client.requestRefresh(validated.id, validated.revision, "Persist validated Host read", now)
            val token = reading.readState.string("read_token")
            readToken = token
            client.recordRead(reading.id, token, buildJsonObject {
                if (bytes.isEmpty()) {
                    put("status", "empty")
                } else {
                    put("status", "succeeded")
                    put("content_reference", buildJsonObject {
                        put("uri", file.toURI().toString()); put("mime_type", "text/plain")
                        put("byte_count", bytes.size); put("content_version", file.nameWithoutExtension)
                    })
                    put("record_count", contract.recordCount)
                }
                put("observed_at_ms", contract.observedAt); put("schema_version", 2)
            }, now)
        } catch (error: Exception) {
            // An execute/decode failure is not proof the write did not commit.
            // Keep the file if the authoritative result refers to it, or if
            // reconciliation itself is unavailable.
            try {
                val current = client.get(validated.id)
                val reference = current?.readState?.get("content_reference").present()?.jsonObject
                if (reference?.get("uri")?.jsonPrimitive?.content != file.toURI().toString()) {
                    file.delete()
                    val pendingToken = current?.readState?.get("read_token").present()?.jsonPrimitive?.content
                    if (readToken != null && pendingToken == readToken) {
                        client.recordRead(validated.id, readToken, buildJsonObject {
                            put("status", "failed"); put("code", "cache_commit_failed")
                            put("diagnostic", error.message ?: "Cannot commit cached source content")
                        }, System.currentTimeMillis())
                    }
                }
            } catch (reconciliationError: Exception) {
                error.addSuppressed(reconciliationError)
            }
            throw error
        }
        if (bytes.isEmpty()) file.delete()
        client.setEnabled(stored.id, stored.revision, source.enabled, now)
    }

    private fun project(source: CanonicalDataSource): CustomDataSourceConfig {
        val contract = source.validation.contract?.let(::decodeContract)
        val reference = source.readState["content_reference"].present()?.jsonObject
        val content = reference?.let {
            val uri = java.net.URI(it.string("uri"))
            if (uri.scheme != "file") return@let ""
            val file = File(uri).canonicalFile
            // A remote or malformed projection cannot grant arbitrary file access.
            if (file.parentFile != cacheDirectory.canonicalFile || !file.isFile) return@let ""
            if (file.length() > MAX_CACHE_BYTES) return@let ""
            file.readText(Charsets.UTF_8)
        }.orEmpty()
        return CustomDataSourceConfig(
            id = source.id, name = source.name, location = source.configuration.location,
            kind = when (source.kind) {
                "http", "web" -> UnifiedDataSourceKind.Url
                "local_file" -> UnifiedDataSourceKind.File
                "directory" -> UnifiedDataSourceKind.Folder
                "database" -> UnifiedDataSourceKind.Database
                "mcp" -> UnifiedDataSourceKind.Mcp
                else -> UnifiedDataSourceKind.Custom
            },
            enabled = source.defaultEnabled,
            accessPolicy = DataSourceAccessPolicy.valueOf(source.accessPolicy.replaceFirstChar { it.uppercase() }),
            allowCloudContext = source.privacy == "allow-cloud-context",
            maximumAgeMinutes = source.freshnessPolicy.maximumAgeMinutes.toInt(),
            validationContract = contract, cachedContent = content,
            lastSuccessfulReadAt = source.lastSuccessfulReadAtMs,
            canonicalAuthorization = source.authorization.state,
            canonicalObservedAt = source.readState["observed_at_ms"].present()?.jsonPrimitive?.long,
            lastError = if (source.readState.string("status") in setOf("failed", "denied", "unavailable"))
                source.readState["diagnostic"].present()?.jsonPrimitive?.content else source.validation.diagnostic,
            createdAt = source.createdAtMs, updatedAt = source.updatedAtMs, canonicalRevision = source.revision,
        )
    }

    private fun decodeContract(value: JsonObject): DataSourceValidationContract {
        val recipe = value.getValue("readRecipe").jsonObject
        return DataSourceValidationContract(
            sourceId = value.string("sourceId"), connectorId = value.string("connectorId"),
            connectorVersion = value.getValue("connectorVersion").jsonPrimitive.int,
            readerDescription = value.string("readerDescription"), format = "canonical",
            schemaFields = value.getValue("schemaFields").jsonArray.map { it.jsonPrimitive.content },
            redactedSample = value.string("redactedSample"), recordCount = value.getValue("recordCount").jsonPrimitive.int,
            maximumAgeMinutes = value.getValue("maximumAgeMinutes").jsonPrimitive.int,
            validationState = DataSourceValidationState.valueOf(value.string("validationState").replaceFirstChar { it.uppercase() }),
            validatedAt = Instant.parse(value.string("validatedAt")).toEpochMilli(),
            observedAt = Instant.parse(value["observedAt"].present()?.jsonPrimitive?.content ?: value.string("validatedAt")).toEpochMilli(),
            readRecipe = DataSourceReadRecipe(
                method = DataSourceReadMethod.valueOf(recipe.string("method").replaceFirstChar { it.uppercase() }),
                operationId = recipe.string("operationId"),
                configurationKeys = recipe["configurationKeys"]?.jsonArray?.map { it.jsonPrimitive.content }.orEmpty(),
                outputSchemaJson = recipe.getValue("outputSchema").toString(),
            ), validationDigest = value.string("validationDigest"),
        )
    }

    private fun JsonElement?.present(): JsonElement? = takeUnless { it == null || it == JsonNull }
    private fun JsonObject.string(key: String): String = getValue(key).jsonPrimitive.content
    private companion object { const val MAX_CACHE_BYTES = 8 * 1024 * 1024 }
}

internal fun DataSourceValidationContract.toCanonicalContract(): JsonObject = buildJsonObject {
    put("sourceId", sourceId); put("connectorId", connectorId); put("connectorVersion", connectorVersion)
    put("readerDescription", readerDescription)
    val recipe = requireNotNull(readRecipe)
    put("readRecipe", buildJsonObject {
        put("method", recipe.method.name.replaceFirstChar { it.lowercase() }); put("operationId", recipe.operationId)
        put("configurationKeys", JsonArray(recipe.configurationKeys.map(::JsonPrimitive)))
        put("outputSchema", Json.parseToJsonElement(recipe.outputSchemaJson))
    })
    put("schemaFields", JsonArray(schemaFields.map(::JsonPrimitive))); put("redactedSample", redactedSample)
    put("recordCount", recordCount); put("maximumAgeMinutes", maximumAgeMinutes)
    put("validationState", validationState.name.replaceFirstChar { it.lowercase() })
    put("validatedAt", Instant.ofEpochMilli(validatedAt).toString()); put("observedAt", Instant.ofEpochMilli(observedAt).toString())
    put("validationDigest", validationDigest.orEmpty())
}
