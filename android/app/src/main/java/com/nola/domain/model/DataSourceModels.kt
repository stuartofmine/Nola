package com.nola.domain.model

import kotlinx.serialization.Serializable
import java.util.UUID

const val DATA_SOURCE_MINIMUM_AGE_MINUTES = 1
const val DATA_SOURCE_MAXIMUM_AGE_MINUTES = 10_080
const val ANDROID_HEALTH_DATA_SOURCE_ID = "health:android"

fun isHealthDataSourceSelected(selectedDataSourceIds: Collection<String>): Boolean =
    ANDROID_HEALTH_DATA_SOURCE_ID in selectedDataSourceIds

@Serializable
enum class DataSourceAccessPolicy {
    DefaultEnabled,
    ConversationOptIn,
    AskWhenRelevant,
    Blocked
}

@Serializable
enum class DataSourceValidationState {
    Verified,
    VerifiedEmpty,
    Failed
}

@Serializable
enum class DataSourceReadMethod {
    Unknown,
    HttpGet,
    LocalFileRead,
    DirectoryList,
    PlatformApi,
    DatabaseQuery,
    McpTool,
    HostTool,
    Custom
}

@Serializable
data class DataSourceReadRecipe(
    val method: DataSourceReadMethod = DataSourceReadMethod.Unknown,
    val operationId: String = "",
    val configurationKeys: List<String> = emptyList(),
    val outputSchemaJson: String = ""
) {
    val isUsable: Boolean
        get() = method != DataSourceReadMethod.Unknown &&
            operationId.isNotBlank() &&
            configurationKeys.all { it.isNotBlank() } &&
            configurationKeys.distinct().size == configurationKeys.size &&
            outputSchemaJson.isNotBlank()
}

@Serializable
data class DataSourceValidationContract(
    val sourceId: String,
    val connectorId: String = "android.http",
    val connectorVersion: Int = 1,
    val readerDescription: String,
    val format: String,
    val schemaFields: List<String> = emptyList(),
    val redactedSample: String = "",
    val recordCount: Int = 0,
    val maximumAgeMinutes: Int = 60,
    val validationState: DataSourceValidationState = DataSourceValidationState.Verified,
    val validatedAt: Long = System.currentTimeMillis(),
    val observedAt: Long = validatedAt,
    val readRecipe: DataSourceReadRecipe? = null,
    val validationDigest: String? = null
) {
    val isUsable: Boolean
        get() = connectorVersion == 1 &&
            validationState != DataSourceValidationState.Failed &&
            readRecipe?.isUsable == true &&
            !validationDigest.isNullOrBlank()

}

@Serializable
data class CustomDataSourceConfig(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val location: String,
    val kind: UnifiedDataSourceKind = UnifiedDataSourceKind.Url,
    val enabled: Boolean = true,
    val accessPolicy: DataSourceAccessPolicy = DataSourceAccessPolicy.ConversationOptIn,
    val allowCloudContext: Boolean = true,
    val maximumAgeMinutes: Int = 60,
    val validationContract: DataSourceValidationContract? = null,
    val cachedContent: String = "",
    val lastSuccessfulReadAt: Long? = null,
    val lastError: String? = null,
    val createdAt: Long = System.currentTimeMillis(),
    val updatedAt: Long = createdAt,
    val canonicalRevision: Long? = null,
    @kotlinx.serialization.Transient val canonicalAuthorization: String? = null,
    @kotlinx.serialization.Transient val canonicalObservedAt: Long? = null,
) {
    val isUsable: Boolean
        get() = enabled && lastError == null && validationContract?.isUsable == true && cachedContent.isNotBlank()

    fun needsRefresh(now: Long = System.currentTimeMillis()): Boolean {
        val lastRead = lastSuccessfulReadAt ?: return true
        val maximumAgeMillis = maximumAgeMinutes.coerceIn(
            DATA_SOURCE_MINIMUM_AGE_MINUTES,
            DATA_SOURCE_MAXIMUM_AGE_MINUTES
        ) * 60_000L
        return now - lastRead >= maximumAgeMillis
    }
}

@Serializable
data class DataSourceUsageRecord(
    val id: String = UUID.randomUUID().toString(),
    val sourceId: String,
    val sourceTitle: String,
    val consumer: String,
    val purpose: String,
    val conversationId: String? = null,
    val disclosedToCloud: Boolean = false,
    val usedAt: Long = System.currentTimeMillis()
)

@Serializable
enum class UnifiedDataSourceKind {
    File,
    Folder,
    Database,
    Url,
    WebSearch,
    Api,
    Health,
    Calendar,
    Reminder,
    Mcp,
    UploadedDocument,
    WebHost,
    Custom
}

@Serializable
enum class UnifiedDataSourceCapability {
    Readable,
    Searchable,
    Watchable,
    Schedulable,
    Writable,
    Executable,
    Contextual
}

@Serializable
enum class UnifiedDataSourcePrivacyScope {
    LocalStorage,
    SummaryOnly,
    AllowCloudContext,
    AllowAgentTools,
    AllowWriteBack,
    RequiresConfirmation
}

@Serializable
enum class UnifiedDataSourceConnectionState {
    Ready,
    Disabled,
    NeedsPermission,
    Unavailable,
    Error
}

@Serializable
data class UnifiedDataSourcePrivacyPolicy(
    val scopes: Set<UnifiedDataSourcePrivacyScope> = setOf(UnifiedDataSourcePrivacyScope.LocalStorage),
    val rawSamplesLeaveDevice: Boolean = false
)

@Serializable
data class UnifiedDataSource(
    val id: String,
    val name: String,
    val kind: UnifiedDataSourceKind,
    val location: String? = null,
    val capabilities: Set<UnifiedDataSourceCapability> = emptySet(),
    val privacy: UnifiedDataSourcePrivacyPolicy = UnifiedDataSourcePrivacyPolicy(),
    val connectionState: UnifiedDataSourceConnectionState = UnifiedDataSourceConnectionState.Ready,
    val lastSyncedAt: Long? = null,
    val metadata: Map<String, String> = emptyMap()
)

@Serializable
data class DataSourceSnapshot(
    val dataSourceId: String,
    val itemCount: Int = 0,
    val tokenEstimate: Int = 0,
    val capturedAt: Long = System.currentTimeMillis(),
    val cursor: String? = null,
    val summary: String? = null
)

@Serializable
data class ContextPack(
    val id: String,
    val dataSourceIds: List<String>,
    val snapshots: List<DataSourceSnapshot> = emptyList(),
    val tokenEstimate: Int = snapshots.sumOf { it.tokenEstimate },
    val createdAt: Long = System.currentTimeMillis()
)

object DataSourceCatalogBuilder {
    fun forChatContext(
        uploadedDocuments: List<UploadedContent>,
        hostUploadedDocuments: List<UploadedContent>,
        mcpServers: List<McpServerConfig>,
        hostMcpServers: List<McpServerConfig>,
        includeHealthSummary: Boolean = true,
        healthConnectionState: UnifiedDataSourceConnectionState = UnifiedDataSourceConnectionState.NeedsPermission,
        healthLastSyncedAt: Long? = null,
        customDataSources: List<CustomDataSourceConfig> = emptyList()
    ): List<UnifiedDataSource> {
        val localDocuments = uploadedDocuments.mapIndexed { index, document ->
            document.toDocumentDataSource(index = index, origin = "local")
        }
        val hostDocuments = hostUploadedDocuments.mapIndexed { index, document ->
            document.toDocumentDataSource(index = index, origin = "web-host")
        }
        val localMcp = mcpServers.mapIndexed { index, server ->
            server.toMcpDataSource(index = index, origin = "local")
        }
        val hostMcp = hostMcpServers.mapIndexed { index, server ->
            server.toMcpDataSource(index = index, origin = "web-host")
        }
        val health = if (includeHealthSummary) {
            listOf(healthSummary(connectionState = healthConnectionState, lastSyncedAt = healthLastSyncedAt))
        } else {
            emptyList()
        }
        val custom = customDataSources.map(::customSource)
        return localDocuments + hostDocuments + localMcp + hostMcp + health + custom
    }

    fun healthSummary(
        connected: Boolean,
        lastSyncedAt: Long? = null
    ): UnifiedDataSource = healthSummary(
        connectionState = if (connected) {
            UnifiedDataSourceConnectionState.Ready
        } else {
            UnifiedDataSourceConnectionState.NeedsPermission
        },
        lastSyncedAt = lastSyncedAt
    )

    fun healthSummary(
        connectionState: UnifiedDataSourceConnectionState,
        lastSyncedAt: Long? = null
    ): UnifiedDataSource = UnifiedDataSource(
        id = ANDROID_HEALTH_DATA_SOURCE_ID,
        name = "Health Connect",
        kind = UnifiedDataSourceKind.Health,
        capabilities = setOf(
            UnifiedDataSourceCapability.Readable,
            UnifiedDataSourceCapability.Contextual
        ),
        privacy = UnifiedDataSourcePrivacyPolicy(
            scopes = setOf(
                UnifiedDataSourcePrivacyScope.LocalStorage,
                UnifiedDataSourcePrivacyScope.SummaryOnly
            ),
            rawSamplesLeaveDevice = false
        ),
        connectionState = connectionState,
        lastSyncedAt = lastSyncedAt,
        metadata = mapOf(
            "accessPolicy" to DataSourceAccessPolicy.ConversationOptIn.name
        )
    )

    private fun UploadedContent.toDocumentDataSource(index: Int, origin: String): UnifiedDataSource {
        val enabledState = if (enabled) {
            UnifiedDataSourceConnectionState.Ready
        } else {
            UnifiedDataSourceConnectionState.Disabled
        }
        return UnifiedDataSource(
            id = "document:$origin:$index:${stableIdPart(name)}",
            name = name.ifBlank { "Document ${index + 1}" },
            kind = UnifiedDataSourceKind.UploadedDocument,
            location = origin,
            capabilities = setOf(
                UnifiedDataSourceCapability.Readable,
                UnifiedDataSourceCapability.Searchable,
                UnifiedDataSourceCapability.Contextual
            ),
            privacy = UnifiedDataSourcePrivacyPolicy(
                scopes = setOf(
                    UnifiedDataSourcePrivacyScope.LocalStorage,
                    UnifiedDataSourcePrivacyScope.AllowCloudContext
                ),
                rawSamplesLeaveDevice = false
            ),
            connectionState = enabledState,
            metadata = mapOf(
                "kind" to kind,
                "characters" to content.length.toString(),
                "origin" to origin,
                "originIndex" to index.toString()
            )
        )
    }

    private fun McpServerConfig.toMcpDataSource(index: Int, origin: String): UnifiedDataSource {
        val enabledState = if (enabled) {
            UnifiedDataSourceConnectionState.Ready
        } else {
            UnifiedDataSourceConnectionState.Disabled
        }
        return UnifiedDataSource(
            id = "mcp:$origin:$index:${stableIdPart(name)}",
            name = name.ifBlank { "MCP ${index + 1}" },
            kind = UnifiedDataSourceKind.Mcp,
            location = url.ifBlank { origin },
            capabilities = setOf(
                UnifiedDataSourceCapability.Executable,
                UnifiedDataSourceCapability.Contextual
            ),
            privacy = UnifiedDataSourcePrivacyPolicy(
                scopes = setOf(
                    UnifiedDataSourcePrivacyScope.AllowAgentTools,
                    UnifiedDataSourcePrivacyScope.RequiresConfirmation
                ),
                rawSamplesLeaveDevice = false
            ),
            connectionState = enabledState,
            metadata = mapOf(
                "origin" to origin,
                "originIndex" to index.toString()
            )
        )
    }

    private fun customSource(source: CustomDataSourceConfig): UnifiedDataSource {
        val connectionState = when {
            source.lastError != null -> UnifiedDataSourceConnectionState.Error
            source.validationContract?.isUsable == true -> UnifiedDataSourceConnectionState.Ready
            else -> UnifiedDataSourceConnectionState.Unavailable
        }
        val scopes = buildSet {
            add(UnifiedDataSourcePrivacyScope.LocalStorage)
            if (source.allowCloudContext) add(UnifiedDataSourcePrivacyScope.AllowCloudContext)
        }
        return UnifiedDataSource(
            id = "data_source:${source.id}",
            name = source.name,
            kind = source.kind,
            location = source.location,
            capabilities = setOf(
                UnifiedDataSourceCapability.Readable,
                UnifiedDataSourceCapability.Contextual,
                UnifiedDataSourceCapability.Watchable
            ),
            privacy = UnifiedDataSourcePrivacyPolicy(scopes = scopes),
            connectionState = connectionState,
            lastSyncedAt = source.lastSuccessfulReadAt,
            metadata = mapOf(
                "customSourceId" to source.id,
                "accessPolicy" to source.accessPolicy.name,
                "format" to source.validationContract?.format.orEmpty(),
                "reader" to source.validationContract?.readerDescription.orEmpty(),
                "recordCount" to (source.validationContract?.recordCount ?: 0).toString()
            )
        )
    }

    private fun stableIdPart(value: String): String {
        val normalized = value.trim().lowercase()
        if (normalized.isBlank()) return "untitled"
        return normalized
            .map { char ->
                when {
                    char.isLetterOrDigit() -> char
                    else -> '-'
                }
            }
            .joinToString("")
            .replace(Regex("-+"), "-")
            .trim('-')
            .ifBlank { "untitled" }
            .take(48)
    }
}
