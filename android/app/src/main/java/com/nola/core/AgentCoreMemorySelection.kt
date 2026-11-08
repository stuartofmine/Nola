// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed Android wrapper for Core's stateless memory-selection ABI.
// - Boundary: Android owns storage/retrieval; Core owns deterministic policy selection.

@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

@Serializable
enum class AgentCoreMemoryScope {
    @SerialName("turn") TURN,
    @SerialName("conversation") CONVERSATION,
    @SerialName("global") GLOBAL,
}

@Serializable
enum class AgentCoreMemoryRecallMode {
    @SerialName("resident_profile") RESIDENT_PROFILE,
    @SerialName("on_demand") ON_DEMAND,
}

@Serializable
enum class AgentCoreMemoryTokenAccounting {
    @SerialName("host_exact") HOST_EXACT,
    @SerialName("core_estimate") CORE_ESTIMATE,
}

@Serializable
enum class AgentCoreMemoryKind {
    @SerialName("preference") PREFERENCE,
    @SerialName("identity") IDENTITY,
    @SerialName("decision") DECISION,
    @SerialName("project") PROJECT,
    @SerialName("feedback") FEEDBACK,
    @SerialName("reference") REFERENCE,
    @SerialName("other") OTHER,
}

@Serializable
enum class AgentCoreMemoryProvenanceKind {
    @SerialName("explicit_user") EXPLICIT_USER,
    @SerialName("agent_inference") AGENT_INFERENCE,
    @SerialName("tool_result") TOOL_RESULT,
    @SerialName("imported") IMPORTED,
    @SerialName("synced") SYNCED,
    @SerialName("other") OTHER,
}

@Serializable
data class AgentCoreMemoryNamespace(
    val kind: String,
    val name: String? = null,
)

@Serializable
data class AgentCoreMemoryRetention(
    val mode: String,
    @SerialName("expires_at") val expiresAt: String? = null,
)

@Serializable
data class AgentCoreMemoryPolicy(
    @SerialName("long_term_enabled") val longTermEnabled: Boolean,
    @SerialName("automatic_extraction_enabled") val automaticExtractionEnabled: Boolean,
    @SerialName("search_limit") val searchLimit: Int,
    @SerialName("minimum_confidence") val minimumConfidence: Float,
    @SerialName("maximum_age_days") val maximumAgeDays: Int? = null,
    @SerialName("allowed_sensitivities") val allowedSensitivities: List<String> = emptyList(),
)

@Serializable
data class AgentCoreMemoryProvenance(
    val kind: AgentCoreMemoryProvenanceKind,
    val source: String,
    @SerialName("session_id") val sessionId: String? = null,
    @SerialName("turn_id") val turnId: String? = null,
    @SerialName("item_ids") val itemIds: List<String> = emptyList(),
    @SerialName("observed_at") val observedAt: String? = null,
)

@Serializable
data class AgentCoreMemoryRecord(
    val id: String,
    val owner: kotlinx.serialization.json.JsonObject,
    val namespace: AgentCoreMemoryNamespace,
    val scope: AgentCoreMemoryScope,
    val kind: AgentCoreMemoryKind,
    val content: String,
    val summary: String? = null,
    val tags: List<String> = emptyList(),
    val confidence: Float,
    val importance: Float,
    val sensitivity: String? = null,
    val retention: AgentCoreMemoryRetention,
    val supersedes: List<String> = emptyList(),
    val provenance: AgentCoreMemoryProvenance,
)

@Serializable
data class AgentCoreMemorySelectionInput(
    val policy: AgentCoreMemoryPolicy,
    val records: List<AgentCoreMemoryRecord>,
    val mode: AgentCoreMemoryRecallMode = AgentCoreMemoryRecallMode.ON_DEMAND,
    @SerialName("allowed_namespaces")
    val allowedNamespaces: List<AgentCoreMemoryNamespace> = emptyList(),
    @SerialName("include_session_retention")
    val includeSessionRetention: Boolean = false,
    val now: String? = null,
    @SerialName("maximum_input_tokens")
    val maximumInputTokens: Int? = null,
    @SerialName("token_accounting")
    val tokenAccounting: AgentCoreMemoryTokenAccounting = AgentCoreMemoryTokenAccounting.HOST_EXACT,
    @SerialName("token_counts")
    val tokenCounts: Map<String, Int> = emptyMap(),
)

@Serializable
enum class AgentCoreMemorySelectionReason {
    @SerialName("resident_profile") RESIDENT_PROFILE,
    @SerialName("host_ranked_on_demand") HOST_RANKED_ON_DEMAND,
}

@Serializable
data class AgentCoreSelectedMemory(
    val record: AgentCoreMemoryRecord,
    @SerialName("requires_current_state_verification")
    val requiresCurrentStateVerification: Boolean,
    @SerialName("token_count") val tokenCount: Int? = null,
    @SerialName("token_count_is_estimated") val tokenCountIsEstimated: Boolean = false,
    @SerialName("selection_reason") val selectionReason: AgentCoreMemorySelectionReason,
)

@Serializable
data class AgentCoreRejectedMemory(
    @SerialName("memory_id") val memoryId: String,
    val reason: String,
)

@Serializable
data class AgentCoreMemorySelectionPlan(
    val mode: AgentCoreMemoryRecallMode,
    val selected: List<AgentCoreSelectedMemory> = emptyList(),
    val rejected: List<AgentCoreRejectedMemory> = emptyList(),
    @SerialName("used_input_tokens") val usedInputTokens: Int,
    @SerialName("remaining_input_tokens") val remainingInputTokens: Int? = null,
    @SerialName("token_counts_are_estimated") val tokenCountsAreEstimated: Boolean = false,
    @SerialName("plan_digest") val planDigest: String,
)

object AgentCoreMemorySelection {
    private val json = Json {
        ignoreUnknownKeys = false
        explicitNulls = false
        encodeDefaults = true
    }

    val isAvailable: Boolean
        get() = AgentCoreNative.isMemorySelectionAvailable

    fun select(input: AgentCoreMemorySelectionInput): AgentCoreMemorySelectionPlan? {
        if (!isAvailable) return null
        val encoded = json.encodeToString(input).encodeToByteArray()
        val output = AgentCoreNative.nativeMemorySelect(encoded) ?: return null
        return json.decodeFromString(output.decodeToString())
    }
}
