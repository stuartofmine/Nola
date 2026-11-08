package com.nola.data.local

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.json.*

/** Application owns lifecycle and revisions; callers own actual source I/O. */
@Serializable
internal data class CanonicalDataSourceConfiguration(
    @SerialName("connector_id") val connectorId: String,
    @SerialName("location_kind") val locationKind: String,
    val location: String,
    val parameters: JsonObject = buildJsonObject {},
    @SerialName("secret_references") val secretReferences: List<String> = emptyList(),
)

@Serializable
internal data class CanonicalDataSourceAuthorization(
    val state: String,
    val capability: String? = null,
    @SerialName("observed_at_ms") val observedAtMs: Long,
)

@Serializable
internal data class CanonicalDataSourceFreshness(
    @SerialName("maximum_age_minutes") val maximumAgeMinutes: Long,
    @SerialName("allow_stale_fallback") val allowStaleFallback: Boolean,
)

@Serializable
internal data class CanonicalDataSourceDraft(
    val id: String,
    val name: String,
    val kind: String,
    val configuration: CanonicalDataSourceConfiguration,
    @SerialName("access_policy") val accessPolicy: String,
    val authorization: CanonicalDataSourceAuthorization,
    @SerialName("freshness_policy") val freshnessPolicy: CanonicalDataSourceFreshness,
    val capabilities: List<String>,
    val privacy: String,
    @SerialName("created_at_ms") val createdAtMs: Long,
    @SerialName("updated_at_ms") val updatedAtMs: Long,
)

@Serializable
internal data class CanonicalDataSourceValidation(
    val status: String,
    @SerialName("validation_token") val validationToken: String?,
    val contract: JsonObject?,
    val diagnostic: String?,
    @SerialName("attempted_at_ms") val attemptedAtMs: Long?,
    @SerialName("completed_at_ms") val completedAtMs: Long?,
)

@Serializable
internal data class CanonicalDataSource(
    val id: String,
    val name: String,
    val kind: String,
    val configuration: CanonicalDataSourceConfiguration,
    @SerialName("access_policy") val accessPolicy: String,
    val authorization: CanonicalDataSourceAuthorization,
    val validation: CanonicalDataSourceValidation,
    @SerialName("read_state") val readState: JsonObject,
    @SerialName("freshness_policy") val freshnessPolicy: CanonicalDataSourceFreshness,
    @SerialName("default_enabled") val defaultEnabled: Boolean,
    val lifecycle: String,
    val capabilities: List<String>,
    val privacy: String,
    val revision: Long,
    @SerialName("last_successful_read_at_ms") val lastSuccessfulReadAtMs: Long?,
    @SerialName("created_at_ms") val createdAtMs: Long,
    @SerialName("updated_at_ms") val updatedAtMs: Long,
    @SerialName("deleted_at_ms") val deletedAtMs: Long?,
)

internal class CanonicalDataSourceClient(private val execute: (JsonObject) -> JsonElement) {
    private val json = Json { encodeDefaults = true; ignoreUnknownKeys = false }

    private fun command(type: String, fields: JsonObjectBuilder.() -> Unit = {}): JsonElement =
        execute(buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject { put("type", type); fields() })
        })

    private fun source(type: String, fields: JsonObjectBuilder.() -> Unit): CanonicalDataSource =
        json.decodeFromJsonElement(command(type, fields))

    fun get(id: String): CanonicalDataSource? = command("get") { put("id", id) }
        .takeUnless { it == JsonNull }?.let { json.decodeFromJsonElement(it) }

    fun list(): List<CanonicalDataSource> = json.decodeFromJsonElement(command("list"))

    fun create(draft: CanonicalDataSourceDraft): CanonicalDataSource = source("create_draft") {
        put("input", json.encodeToJsonElement(draft))
    }

    fun update(draft: CanonicalDataSourceDraft, expectedRevision: Long): CanonicalDataSource =
        source("update_configuration") {
            put("id", draft.id)
            put("expected_revision", expectedRevision)
            val input = json.encodeToJsonElement(draft).jsonObject
            put("input", JsonObject(input.filterKeys { it != "id" && it != "created_at_ms" }))
        }

    fun requestValidation(id: String, expectedRevision: Long, now: Long): CanonicalDataSource =
        source("request_validation") {
            put("id", id); put("expected_revision", expectedRevision); put("requested_at_ms", now)
        }

    fun recordValidation(id: String, token: String, result: JsonObject, now: Long): CanonicalDataSource =
        source("record_validation") {
            put("id", id); put("validation_token", token); put("result", result); put("recorded_at_ms", now)
        }

    fun setEnabled(id: String, expectedRevision: Long, enabled: Boolean, now: Long): CanonicalDataSource =
        source("set_default_enabled") {
            put("id", id); put("expected_revision", expectedRevision)
            put("enabled", enabled); put("updated_at_ms", now)
        }

    fun delete(id: String, expectedRevision: Long, now: Long) {
        command("delete") {
            put("id", id); put("expected_revision", expectedRevision); put("deleted_at_ms", now)
        }
    }

    fun requestRefresh(id: String, expectedRevision: Long, reason: String, now: Long): CanonicalDataSource =
        source("request_refresh") {
            put("id", id); put("expected_revision", expectedRevision)
            put("reason", reason); put("requested_at_ms", now)
        }

    fun recordRead(id: String, token: String, result: JsonObject, now: Long): CanonicalDataSource =
        source("record_read") {
            put("id", id); put("read_token", token); put("result", result); put("recorded_at_ms", now)
        }
}
