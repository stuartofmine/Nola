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
    val status: String = "not_validated",
    @SerialName("validation_token") val validationToken: String? = null,
    val contract: JsonObject? = null,
    val diagnostic: String? = null,
    @SerialName("attempted_at_ms") val attemptedAtMs: Long? = null,
    @SerialName("completed_at_ms") val completedAtMs: Long? = null,
)

@Serializable
internal data class CanonicalDataSource(
    val id: String,
    val name: String,
    val kind: String,
    val configuration: CanonicalDataSourceConfiguration,
    @SerialName("access_policy") val accessPolicy: String,
    val authorization: CanonicalDataSourceAuthorization,
    val validation: CanonicalDataSourceValidation = CanonicalDataSourceValidation(),
    @SerialName("read_state") val readState: JsonObject = buildJsonObject {
        put("status", "never_read")
        put("read_token", JsonNull)
    },
    @SerialName("freshness_policy") val freshnessPolicy: CanonicalDataSourceFreshness,
    @SerialName("default_enabled") val defaultEnabled: Boolean = true,
    val lifecycle: String = "ready",
    val capabilities: List<String> = listOf("read", "context"),
    val privacy: String,
    val revision: Long = 1L,
    @SerialName("last_successful_read_at_ms") val lastSuccessfulReadAtMs: Long? = null,
    @SerialName("created_at_ms") val createdAtMs: Long,
    @SerialName("updated_at_ms") val updatedAtMs: Long,
    @SerialName("deleted_at_ms") val deletedAtMs: Long? = null,
)

internal class CanonicalDataSourceClient(private val execute: (JsonObject) -> JsonElement) {
    private val json = Json { encodeDefaults = true; ignoreUnknownKeys = true }

    private fun command(type: String, fields: JsonObjectBuilder.() -> Unit = {}): JsonElement =
        execute(buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject { put("type", type); fields() })
        })

    private fun decodeSourceOrNull(raw: JsonElement): CanonicalDataSource? {
        val obj = raw as? JsonObject ?: return null
        val candidate = (obj["record"] as? JsonObject) ?: obj
        if (!candidate.containsKey("configuration")) return null
        return json.decodeFromJsonElement(CanonicalDataSource.serializer(), candidate)
    }

    private fun unwrapSource(raw: JsonElement, fallbackId: String? = null): CanonicalDataSource {
        decodeSourceOrNull(raw)?.let { return it }
        val targetId = fallbackId
            ?: (raw as? JsonObject)?.get("id")?.jsonPrimitive?.contentOrNull
        if (targetId != null && (raw as? JsonObject)?.get("ok")?.jsonPrimitive?.booleanOrNull == true) {
            return requireNotNull(get(targetId)) { "Data source '$targetId' was not found after mutation" }
        }
        val candidate = ((raw as? JsonObject)?.get("record") as? JsonObject) ?: raw
        return json.decodeFromJsonElement(CanonicalDataSource.serializer(), candidate)
    }

    private fun source(type: String, fallbackId: String? = null, fields: JsonObjectBuilder.() -> Unit): CanonicalDataSource =
        unwrapSource(command(type, fields), fallbackId)

    fun get(id: String): CanonicalDataSource? = command("get") { put("id", id) }
        .takeUnless { it == JsonNull }?.let { unwrapSource(it) }

    fun list(): List<CanonicalDataSource> =
        command("list").extractJsonArray("sources", "data_sources", "items", "records")
            .map { unwrapSource(it) }

    fun create(draft: CanonicalDataSourceDraft): CanonicalDataSource = source("create_draft", fallbackId = draft.id) {
        val draftFields = json.encodeToJsonElement(draft).jsonObject
        put("input", JsonObject(draftFields + mapOf(
            "revision" to JsonPrimitive(1L),
            "default_enabled" to JsonPrimitive(true),
            "lifecycle" to JsonPrimitive("draft"),
            "validation" to json.encodeToJsonElement(CanonicalDataSourceValidation()),
            "read_state" to buildJsonObject {
                put("status", "never_read")
                put("read_token", JsonNull)
            },
        )))
    }

    fun update(draft: CanonicalDataSourceDraft, expectedRevision: Long): CanonicalDataSource =
        source("update_configuration", fallbackId = draft.id) {
            put("id", draft.id)
            put("expected_revision", expectedRevision)
            val recordFields = json.encodeToJsonElement(draft).jsonObject
                .filterKeys { it != "id" && it != "created_at_ms" } +
                ("revision" to JsonPrimitive(expectedRevision + 1L))
            // Nest under "record" so v2 daemon update_configuration merges all top-level fields
            // instead of extracting only the inner "configuration" sub-object.
            put("input", buildJsonObject {
                put("id", draft.id)
                put("record", JsonObject(recordFields))
            })
        }

    fun requestValidation(id: String, expectedRevision: Long, now: Long): CanonicalDataSource {
        val raw = command("request_validation") {
            put("id", id); put("expected_revision", expectedRevision); put("requested_at_ms", now)
        }
        decodeSourceOrNull(raw)?.let { return it }
        val current = unwrapSource(raw, fallbackId = id)
        val token = current.validation.validationToken ?: "val-$id-$now"
        return current.copy(
            validation = current.validation.copy(
                status = "pending",
                validationToken = token,
                attemptedAtMs = now,
            )
        )
    }

    fun recordValidation(id: String, token: String, result: JsonObject, now: Long): CanonicalDataSource {
        val raw = command("record_validation") {
            put("id", id)
            put("validation_token", token)
            put("result", result)
            put("recorded_at_ms", now)
            // Mirror canonical validation fields at top level so v2 daemon persists them on record.validation.
            put("status", result["status"] ?: JsonPrimitive("succeeded"))
            result["contract"]?.let { put("contract", it) }
            result["diagnostic"]?.let { put("diagnostic", it) }
            put("completed_at_ms", now)
        }
        decodeSourceOrNull(raw)?.let { return it }
        return unwrapSource(raw, fallbackId = id)
    }

    fun setEnabled(id: String, expectedRevision: Long, enabled: Boolean, now: Long): CanonicalDataSource =
        source("set_default_enabled", fallbackId = id) {
            put("id", id); put("expected_revision", expectedRevision)
            put("enabled", enabled); put("updated_at_ms", now)
        }

    fun delete(id: String, expectedRevision: Long, now: Long) {
        command("delete") {
            put("id", id); put("expected_revision", expectedRevision); put("deleted_at_ms", now)
        }
    }

    fun requestRefresh(id: String, expectedRevision: Long, reason: String, now: Long): CanonicalDataSource {
        val raw = command("request_refresh") {
            put("id", id); put("expected_revision", expectedRevision)
            put("reason", reason); put("requested_at_ms", now)
        }
        decodeSourceOrNull(raw)?.let { return it }
        val current = unwrapSource(raw, fallbackId = id)
        val token = current.readState["read_token"]?.jsonPrimitive?.contentOrNull ?: "read-$id-$now"
        return current.copy(
            readState = JsonObject(current.readState + mapOf(
                "status" to JsonPrimitive("pending"),
                "read_token" to JsonPrimitive(token),
            ))
        )
    }

    fun recordRead(id: String, token: String, result: JsonObject, now: Long): CanonicalDataSource {
        val raw = command("record_read") {
            put("id", id); put("read_token", token); put("result", result); put("recorded_at_ms", now)
        }
        decodeSourceOrNull(raw)?.let { return it }
        val status = result["status"]?.jsonPrimitive?.contentOrNull
        val observedAt = result["observed_at_ms"]?.jsonPrimitive?.longOrNull
        val persistedReadState = JsonObject(result + ("recorded_at_ms" to JsonPrimitive(now)))
        val current = unwrapSource(raw, fallbackId = id)
        return source("update_configuration", fallbackId = id) {
            put("id", id)
            put("expected_revision", current.revision)
            put("input", buildJsonObject {
                put("id", id)
                put("record", buildJsonObject {
                    put("read_state", persistedReadState)
                    put("lifecycle", if (status == "succeeded" || status == "empty") "ready" else current.lifecycle)
                    if (status == "succeeded") {
                        put("last_successful_read_at_ms", observedAt ?: now)
                    }
                    put("updated_at_ms", now)
                })
            })
        }
    }
}
