@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

@Serializable
data class IntelligenceEvent(
    val id: String = "",
    val source: String,
    val kind: String,
    val occurred_at: String = "",
    val observed_at: String = "",
    val subject: String? = null,
    val summary: String,
    val payload: JsonObject? = null,
    val importance: Double = 0.5,
    val confidence: Double = 0.7,
    val tags: List<String> = emptyList(),
    val dedupe_key: String? = null
)

@Serializable
data class IntelligenceMemoryCandidate(
    val semantic_key: String? = null,
    val kind: String = "semantic",
    val statement: String,
    val confidence: Double = 0.7,
    val source_event_ids: List<String> = emptyList(),
    val observed_at: String = "",
    val tags: List<String> = emptyList()
)

@Serializable
data class IntelligenceTaskOutcome(
    val id: String = "",
    val task_type: String,
    val route: String,
    val model: String,
    val tools: List<String> = emptyList(),
    val success: Boolean,
    val user_corrected: Boolean = false,
    val retry_count: Int = 0,
    val latency_ms: Long = 0,
    val occurred_at: String = ""
)

@Serializable
data class IntelligenceIngestRequest(
    val events: List<IntelligenceEvent> = emptyList(),
    val memory_candidates: List<IntelligenceMemoryCandidate> = emptyList(),
    val outcomes: List<IntelligenceTaskOutcome> = emptyList()
)

@Serializable
data class IntelligenceRecallRequest(
    val query: String,
    val limit: Int = 8,
    val kinds: List<String> = emptyList(),
    val source: String? = null,
    val minimum_score: Double = 0.18
)

object PersonalIntelligenceCore {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = false
    }

    val isAvailable: Boolean
        get() = AgentCoreNative.isIntelligenceAvailable

    fun ingest(request: IntelligenceIngestRequest): JsonObject? = call(
        json.encodeToString(request),
        AgentCoreNative::nativeIntelligenceIngest
    )

    fun recall(request: IntelligenceRecallRequest): JsonObject? = call(
        json.encodeToString(request),
        AgentCoreNative::nativeIntelligenceRecall
    )

    fun recallContext(query: String, limit: Int = 6): String =
        recall(IntelligenceRecallRequest(query = query, limit = limit))
            ?.get("context")
            ?.jsonPrimitive
            ?.contentOrNull
            .orEmpty()

    fun snapshot(): JsonObject? {
        if (!isAvailable) return null
        val bytes = AgentCoreNative.nativeIntelligenceSnapshot() ?: return null
        return runCatching { json.parseToJsonElement(bytes.decodeToString()) as? JsonObject }.getOrNull()
    }

    fun clear(): Boolean = isAvailable && AgentCoreNative.nativeIntelligenceClear()

    private fun call(payload: String, function: (ByteArray) -> ByteArray?): JsonObject? {
        if (!isAvailable) return null
        val bytes = function(payload.encodeToByteArray()) ?: return null
        return runCatching { json.parseToJsonElement(bytes.decodeToString()) as? JsonObject }.getOrNull()
    }
}
