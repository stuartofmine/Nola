@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

const val AGENT_CORE_DATA_SOURCE_SCHEMA_VERSION = 1
const val AGENT_CORE_DATA_SOURCE_ABI_VERSION = 1

@Serializable
data class AgentCoreDataSourceResolutionInput(
    val schemaVersion: Int = AGENT_CORE_DATA_SOURCE_SCHEMA_VERSION,
    val sourceId: String,
    val policy: String,
    val conversationOverride: Boolean? = null,
    val contract: JsonObject? = null,
    val now: String? = null,
)

@Serializable
data class AgentCoreDataSourceResolution(
    val schemaVersion: Int,
    val enabled: Boolean,
    val usable: Boolean,
    val shouldRequestConsent: Boolean,
    val shouldRefresh: Boolean,
    val reason: String,
)

object AgentCoreDataSourceResolver {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    val isAvailable: Boolean
        get() = runCatching {
            AgentCoreNative.nativeDataSourceAbiVersion() == AGENT_CORE_DATA_SOURCE_ABI_VERSION
        }.getOrDefault(false)

    internal fun encodeInput(input: AgentCoreDataSourceResolutionInput): ByteArray =
        json.encodeToString(input).encodeToByteArray()

    internal fun decodeOutput(output: ByteArray): AgentCoreDataSourceResolution =
        json.decodeFromString(AgentCoreDataSourceResolution.serializer(), output.decodeToString())

    fun resolve(input: AgentCoreDataSourceResolutionInput): AgentCoreDataSourceResolution? {
        if (!isAvailable) return null
        val encoded = encodeInput(input)
        val output = AgentCoreNative.nativeDataSourceResolve(encoded) ?: return null
        return runCatching { decodeOutput(output) }.getOrNull()
    }

    fun sealContract(contract: JsonObject): JsonObject? {
        if (!isAvailable) return null
        val encoded = json.encodeToString(JsonObject.serializer(), contract).encodeToByteArray()
        val output = AgentCoreNative.nativeDataSourceSealContract(encoded) ?: return null
        return runCatching {
            json.decodeFromString(JsonObject.serializer(), output.decodeToString())
        }.getOrNull()
    }
}
