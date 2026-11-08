package com.nola.data.repository

import io.ktor.client.request.prepareGet
import io.ktor.client.statement.bodyAsChannel
import io.ktor.http.charset
import io.ktor.utils.io.readRemaining
import io.ktor.utils.io.core.readBytes

import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.core.AgentCoreDataSourceResolutionInput
import com.nola.core.AgentCoreDataSourceResolution
import com.nola.core.AgentCoreDataSourceResolver
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.DataSourceValidationContract
import com.nola.domain.model.DataSourceValidationState
import com.nola.domain.model.DATA_SOURCE_MAXIMUM_AGE_MINUTES
import com.nola.domain.model.DATA_SOURCE_MINIMUM_AGE_MINUTES
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.get
import io.ktor.client.statement.HttpResponse
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.isSuccess
import java.net.URI
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.time.Instant

data class DataSourceReadResult(
    val content: String,
    val contract: DataSourceValidationContract
)

internal fun selectUsableDataSources(
    sources: List<CustomDataSourceConfig>,
    selectedIds: Collection<String>,
    disclosesToCloud: Boolean,
    resolve: (CustomDataSourceConfig, Set<String>) -> AgentCoreDataSourceResolution? =
        { source, selected -> resolveDataSource(source, selected) }
): List<CustomDataSourceConfig> {
    val selected = selectedIds.toSet()
    return sources.filter { source ->
        source.enabled && source.cachedContent.isNotBlank() && source.lastError == null &&
            (!disclosesToCloud || source.allowCloudContext) &&
            resolve(source, selected)?.usable == true
    }
}

private fun resolveDataSource(
    source: CustomDataSourceConfig,
    selectedIds: Set<String>,
    now: Long = System.currentTimeMillis()
) = AgentCoreDataSourceResolver.resolve(
    AgentCoreDataSourceResolutionInput(
        sourceId = source.id,
        policy = source.accessPolicy.name.replaceFirstChar { it.lowercase() },
        conversationOverride = when (source.accessPolicy) {
            DataSourceAccessPolicy.DefaultEnabled -> null
            else -> source.id in selectedIds
        },
        contract = source.validationContract?.toCoreContract(),
        now = Instant.ofEpochMilli(now).toString()
    )
)

private fun DataSourceValidationContract.toCoreContract(): JsonObject = buildJsonObject {
    put("sourceId", sourceId)
    put("connectorId", connectorId)
    put("connectorVersion", connectorVersion)
    put("readerDescription", readerDescription)
    put("readRecipe", buildJsonObject {
        val recipe = readRecipe
        put("method", recipe?.method?.name?.replaceFirstChar { it.lowercase() } ?: "unknown")
        put("operationId", recipe?.operationId.orEmpty())
        put("configurationKeys", buildJsonArray {
            recipe?.configurationKeys.orEmpty().forEach { add(JsonPrimitive(it)) }
        })
        put(
            "outputSchema",
            recipe?.outputSchemaJson
                ?.let { runCatching { Json.parseToJsonElement(it) }.getOrNull() }
                ?: buildJsonObject { put("type", "object") }
        )
    })
    put("schemaFields", buildJsonArray { schemaFields.forEach { add(JsonPrimitive(it)) } })
    put("redactedSample", redactedSample)
    put("recordCount", recordCount)
    put("maximumAgeMinutes", maximumAgeMinutes)
    put("validationState", validationState.name.replaceFirstChar { it.lowercase() })
    put("validatedAt", Instant.ofEpochMilli(validatedAt).toString())
    put("observedAt", Instant.ofEpochMilli(observedAt).toString())
    put("validationDigest", validationDigest.orEmpty())
}

private fun sealDataSourceValidationContract(
    contract: DataSourceValidationContract,
    recipe: com.nola.domain.model.DataSourceReadRecipe
): DataSourceValidationContract {
    val pending = contract.copy(
        connectorVersion = contract.connectorVersion,
        readRecipe = recipe,
        validationDigest = null
    )
    val sealed = AgentCoreDataSourceResolver.sealContract(pending.toCoreContract())
        ?: return pending
    val digest = sealed["validationDigest"]?.jsonPrimitive?.content.orEmpty()
    return pending.copy(validationDigest = digest.ifBlank { null })
}

internal fun buildDataSourceUsageRecords(
    usedComponentIds: Collection<String>,
    sources: List<CustomDataSourceConfig>,
    conversationId: String,
    consumer: String,
    disclosedToCloud: Boolean,
    usedAt: Long = System.currentTimeMillis()
): List<DataSourceUsageRecord> {
    val sourcesById = sources.associateBy { it.id }
    return usedComponentIds.asSequence()
        .mapNotNull { componentId ->
            componentId.removePrefix("data_source:").takeIf { it != componentId && it.isNotBlank() }
        }
        .distinct()
        .mapNotNull { sourceId ->
            sourcesById[sourceId]?.let { source ->
                DataSourceUsageRecord(
                    sourceId = source.id,
                    sourceTitle = source.name,
                    consumer = consumer,
                    purpose = "Conversation context",
                    conversationId = conversationId,
                    disclosedToCloud = disclosedToCloud,
                    usedAt = usedAt
                )
            }
        }
        .toList()
}

object DataSourceDocumentInspector {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }
    private val secretPattern = Regex(
        "(?i)(\\\"?(?:api[_-]?key|authorization|token|secret|password)\\\"?\\s*[:=]\\s*)(\\\"?)[^\\\",\\s}]+(\\\"?)"
    )

    fun inspect(
        sourceId: String,
        location: String,
        contentType: String?,
        rawContent: String,
        maximumAgeMinutes: Int,
        now: Long = System.currentTimeMillis()
    ): DataSourceReadResult {
        require(rawContent.toByteArray(Charsets.UTF_8).size <= 1_000_000) { "Data source response exceeds the 1 MB limit" }
        val raw = rawContent.trim()
        require(raw.isNotBlank()) { "Data source returned no readable content" }
        val mediaType = contentType.orEmpty().lowercase()
        val parsedJson = if ("json" in mediaType || raw.startsWith("{") || raw.startsWith("[")) {
            runCatching { json.parseToJsonElement(raw) }.getOrNull()
        } else null
        val isHtml = "html" in mediaType || raw.startsWith("<!doctype", true) || raw.startsWith("<html", true)
        val isCsv = !isHtml && parsedJson == null && raw.lineSequence().firstOrNull()?.count { it == ',' }?.let { it > 0 } == true
        val normalized = when {
            isHtml -> raw
                .replace(Regex("(?is)<script.*?</script>|<style.*?</style>"), " ")
                .replace(Regex("(?s)<[^>]+>"), " ")
                .replace("&nbsp;", " ")
                .replace("&amp;", "&")
                .replace(Regex("\\s+"), " ")
                .trim()
            else -> raw
        }
        require(normalized.isNotBlank()) { "Data source returned no readable content" }
        val format = when {
            parsedJson != null -> "JSON"
            isCsv -> "CSV"
            isHtml -> "HTML text"
            else -> "Plain text"
        }
        val schema = when (parsedJson) {
            is JsonObject -> parsedJson.keys.sorted().take(32)
            is JsonArray -> (parsedJson.firstOrNull() as? JsonObject)?.keys?.sorted()?.take(32).orEmpty()
            else -> if (isCsv) normalized.lineSequence().first().split(',').map(String::trim).filter(String::isNotBlank).take(32) else emptyList()
        }
        val records = when (parsedJson) {
            is JsonArray -> parsedJson.size
            else -> if (isCsv) (normalized.lineSequence().count() - 1).coerceAtLeast(1) else 1
        }
        val reader = when (format) {
            "JSON" -> "HTTP GET -> JSON -> RawEvent"
            "CSV" -> "HTTP GET -> CSV -> RawEvent"
            "HTML text" -> "HTTP GET -> HTML text -> RawEvent"
            else -> "HTTP GET -> text -> RawEvent"
        }
        val sample = secretPattern.replace(normalized.take(800)) { match ->
            "${match.groupValues[1]}${match.groupValues[2]}[REDACTED]${match.groupValues[3]}"
        }
        val outputSchemaJson = when (format) {
            "CSV" -> "{\"type\":\"array\",\"items\":{\"type\":\"object\",\"properties\":{${
                schema.joinToString(",") { field -> "\"${jsonEscaped(field)}\":{\"type\":\"string\"}" }
            }}}}"
            "JSON" -> if (schema.isEmpty()) {
                "{\"type\":\"object\"}"
            } else {
                "{\"type\":\"object\",\"properties\":{${
                    schema.joinToString(",") { field -> "\"${jsonEscaped(field)}\":{}" }
                }}}"
            }
            else -> "{\"type\":\"string\"}"
        }
        val contract = DataSourceValidationContract(
            sourceId = sourceId,
            readerDescription = reader,
            format = format,
            schemaFields = schema,
            redactedSample = sample,
            recordCount = records,
            maximumAgeMinutes = maximumAgeMinutes.coerceIn(
                DATA_SOURCE_MINIMUM_AGE_MINUTES,
                DATA_SOURCE_MAXIMUM_AGE_MINUTES
            ),
            validationState = DataSourceValidationState.Verified,
            validatedAt = now,
            observedAt = now
        )
        val sealedContract = sealDataSourceValidationContract(
            contract,
            com.nola.domain.model.DataSourceReadRecipe(
                method = com.nola.domain.model.DataSourceReadMethod.HttpGet,
                operationId = "android.http.get",
                configurationKeys = listOf("url", "maximumBytes"),
                outputSchemaJson = outputSchemaJson
            )
        )
        return DataSourceReadResult(
            content = normalized,
            contract = sealedContract
        )
    }

    private fun jsonEscaped(value: String): String = buildString(value.length) {
        value.forEach { character ->
            when (character) {
                '\\' -> append("\\\\")
                '"' -> append("\\\"")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> append(character)
            }
        }
    }
}

@Singleton
class AndroidDataSourceReader @Inject constructor(
    private val client: HttpClient
) {
    suspend fun read(source: CustomDataSourceConfig): DataSourceReadResult = kotlinx.coroutines.withTimeoutOrNull(20_000) {
        val location = source.location.trim()
        val uri = runCatching { URI(location) }.getOrNull()
        require(uri != null && uri.scheme?.lowercase() in setOf("http", "https") && !uri.host.isNullOrBlank()) {
            "Enter a valid HTTP or HTTPS address"
        }
        client.prepareGet(location).execute { response ->
            require(response.status.isSuccess()) { "Data source returned HTTP ${response.status.value}" }
            val declaredBytes = response.headers["Content-Length"]?.toLongOrNull()
            require(declaredBytes == null || declaredBytes <= 1_000_000L) {
                "Data source response exceeds the 1 MB limit"
            }
            // execute scopes the streaming response; never materialize an
            // unbounded body before enforcing the limit, including chunked data.
            val bytes = response.bodyAsChannel().readRemaining(limit = 1_000_001L).readBytes()
            require(bytes.size <= 1_000_000) { "Data source response exceeds the 1 MB limit" }
            val contentType = response.headers["Content-Type"]
            val charset = contentType?.let { io.ktor.http.ContentType.parse(it).charset() } ?: Charsets.UTF_8
            DataSourceDocumentInspector.inspect(
                sourceId = source.id,
                location = source.location,
                contentType = contentType,
                rawContent = bytes.toString(charset),
                maximumAgeMinutes = source.maximumAgeMinutes
            )
        }
    } ?: error("Data source request timed out")
}

@Singleton
class AndroidDataSourceRepository @Inject constructor(
    private val store: MemoryProfileDataStore,
    private val reader: AndroidDataSourceReader
) {
    val sources: Flow<List<CustomDataSourceConfig>> = store.profile.map { it.customDataSources }
    val usage: Flow<List<DataSourceUsageRecord>> = store.profile.map { it.dataSourceUsageRecords }

    suspend fun snapshot(): Pair<List<CustomDataSourceConfig>, List<DataSourceUsageRecord>> {
        val profile = store.getSnapshot()
        return profile.customDataSources to profile.dataSourceUsageRecords
    }

    suspend fun validateAndUpsert(draft: CustomDataSourceConfig): CustomDataSourceConfig {
        require(draft.name.trim().isNotBlank()) { "Data source name is required" }
        val result = reader.read(draft)
        val profile = store.getSnapshot()
        val previous = profile.customDataSources.firstOrNull { it.id == draft.id }
        val saved = draft.copy(
            name = draft.name.trim(),
            validationContract = result.contract,
            cachedContent = result.content,
            lastSuccessfulReadAt = result.contract.observedAt,
            lastError = null,
            createdAt = previous?.createdAt ?: draft.createdAt,
            updatedAt = System.currentTimeMillis()
        )
        store.saveDataSource(saved)
        return store.getSnapshot().customDataSources.first { it.id == saved.id }
    }

    suspend fun delete(sourceId: String) {
        store.deleteDataSource(sourceId)
    }

    suspend fun setEnabled(sourceId: String, enabled: Boolean) {
        store.setDataSourceEnabled(sourceId, enabled)
    }

    suspend fun refreshAll() {
        val profile = store.getSnapshot()
        profile.customDataSources.forEach { source -> refreshSource(source) }
    }

    suspend fun sourcesForConversation(
        conversationId: String,
        disclosesToCloud: Boolean
    ): List<CustomDataSourceConfig> {
        val profile = store.getSnapshot()
        val selections = store.resolveDataSources(conversationId).associateBy { it.sourceId }
        val allowedBeforeRefresh = profile.customDataSources.filter { source ->
            val selection = selections[source.id]
            selection?.enabled == true && selection.sourceRevision == source.canonicalRevision &&
                (!disclosesToCloud || source.allowCloudContext)
        }
        allowedBeforeRefresh.forEach { source ->
            val resolution = selections[source.id]
            if (resolution?.shouldRefresh == true) {
                refreshSource(source, enabledForRead = resolution.enabled)
            }
        }
        val current = store.getSnapshot().customDataSources
        val resolved = store.resolveDataSources(conversationId).associateBy { it.sourceId }
        return current.filter { source ->
            val selection = resolved[source.id]
            selection?.enabled == true && selection.usable &&
                selection.sourceRevision == source.canonicalRevision &&
                (!disclosesToCloud || source.allowCloudContext) && source.cachedContent.isNotBlank()
        }
    }

    private suspend fun refreshSource(
        source: CustomDataSourceConfig,
        enabledForRead: Boolean = source.enabled
    ): CustomDataSourceConfig {
        if (!enabledForRead || source.validationContract == null) return source
        return store.refreshDataSource(source) { refreshDataSourceContent(source, enabledForRead, reader::read) }
    }

    suspend fun recordUsage(records: List<DataSourceUsageRecord>) {
        store.recordDataSourceUsage(records)
    }
}
