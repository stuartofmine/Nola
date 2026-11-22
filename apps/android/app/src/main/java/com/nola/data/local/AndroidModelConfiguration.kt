package com.nola.data.local

import com.nola.data.local.datastore.ModelParametersStore
import com.nola.domain.model.RemoteModelEndpointURL
import com.nola.domain.model.ModelExecutionCapabilities
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import java.net.URI
import java.util.UUID

/** API configuration is a projection of Application-owned model profiles. */
class AndroidModelConfiguration(private val runtime: AndroidApplicationRuntime) {
    data class Snapshot(val endpoint: String, val model: String, val wireProtocol: String) {
        val supportsDeviceProvider: Boolean get() = wireProtocol == "chat_completions" || wireProtocol == "responses"
    }

    private val json = Json { encodeDefaults = true; ignoreUnknownKeys = true }

    private fun profiles() = runtime.listModelProfiles().filter {
        it.provider == "openai-compatible" && it.runtime == "remote" && it.enabled
    }

    fun configuredProfiles(): List<CanonicalModelProfile> = profiles()

    fun updateExecutionCapabilities(profileId: String, expectedRevision: Long, capabilities: ModelExecutionCapabilities) {
        val catalog = runtime.listModelProfiles().filter {
            it.provider == "openai-compatible" && it.runtime == "remote" && it.deletedAtMs == null
        }
        val current = catalog.firstOrNull { it.id == profileId }
            ?: error("The configured model is no longer available")
        require(current.revision == expectedRevision) { "The model changed; reload before editing capabilities" }
        require(source(current).wireProtocol in setOf("chat_completions", "responses")) {
            "This device cannot edit execution capabilities for another protocol"
        }
        val fields = json.parseToJsonElement(current.sourceJson).jsonObject.toMutableMap()
        fields["executionCapabilities"] = json.encodeToJsonElement(capabilities)
        val updated = current.copy(sourceJson = JsonObject(fields).toString(), updatedAtMs = System.currentTimeMillis())
        runtime.replaceModelProfiles("openai-compatible", "remote", catalog.map {
            if (it.id == profileId) updated else it
        }, catalog.associate { it.id to it.revision }, updated.updatedAtMs)
    }

    fun load(): Snapshot? {
        val configured = profiles().sortedWith(compareByDescending<CanonicalModelProfile> { it.selected }
            .thenBy { source(it).endpointOrder }.thenBy { source(it).modelOrder }.thenBy { it.id })
        val active = configured.firstOrNull() ?: return null
        val activeSource = source(active)
        val group = configured.filter { groupKey(source(it)) == groupKey(activeSource) }
        require(group.all { source(it).baseURL == activeSource.baseURL }) { "API model endpoint has inconsistent URLs" }
        return Snapshot(activeSource.baseURL, group.joinToString(", ") { it.modelName }, activeSource.wireProtocol)
    }

    fun add(endpoint: String, models: String, wireProtocol: String? = null, credentialReference: String? = null) {
        val normalizedEndpoint = endpoint.trim()
        val names = parseModelNames(models)
        require(normalizedEndpoint.isNotEmpty() && names.isNotEmpty()) {
            "Endpoint and model names must be configured together"
        }
        validateInput(normalizedEndpoint, names)
        require(wireProtocol == null || wireProtocol in setOf("chat_completions", "responses")) {
            "Unsupported model wire protocol"
        }
        val catalog = runtime.listModelProfiles().filter {
            it.provider == "openai-compatible" && it.runtime == "remote" && it.deletedAtMs == null
        }
        val expectedRevisions = catalog.associate { it.id to it.revision }
        val now = System.currentTimeMillis()
        val merged = mergeAddedModelProfiles(
            catalog = catalog,
            normalizedEndpoint = normalizedEndpoint,
            names = names,
            wireProtocol = wireProtocol,
            credentialReference = credentialReference,
            now = now,
        )
        runtime.replaceModelProfiles("openai-compatible", "remote", merged, expectedRevisions, now)
    }

    fun save(endpoint: String, models: String, wireProtocol: String? = null, credentialReference: String? = null) {
        val normalizedEndpoint = endpoint.trim()
        val names = parseModelNames(models)
        validateInput(normalizedEndpoint, names)
        require(wireProtocol == null || wireProtocol in setOf("chat_completions", "responses")) {
            "Unsupported model wire protocol"
        }
        val catalog = runtime.listModelProfiles().filter {
            it.provider == "openai-compatible" && it.runtime == "remote" && it.deletedAtMs == null
        }
        val active = catalog.filter { it.enabled }.firstOrNull { it.selected }
            ?: catalog.firstOrNull { it.enabled }
        val activeSource = active?.let(::source)
        val selectedProtocol = wireProtocol ?: activeSource?.wireProtocol ?: "chat_completions"
        val group = catalog.filter { activeSource != null && groupKey(source(it)) == groupKey(activeSource) }
        // The compact Android editor only lists enabled models. Disabled models
        // in the same endpoint remain part of the canonical catalog until the
        // user explicitly re-adds their name; editing visible models must not
        // silently delete them.
        val untouched = catalog.filter { profile ->
            group.none { it.id == profile.id } || (!profile.enabled && profile.modelName !in names)
        }
        val expectedRevisions = catalog.associate { it.id to it.revision }
        val selected = group.firstOrNull { it.selected && it.modelName in names }?.modelName ?: names.firstOrNull()
        val now = System.currentTimeMillis()
        val endpointId = activeSource?.endpointID?.takeIf { it.isNotBlank() } ?: UUID.randomUUID().toString()
        val defaults = json.encodeToString(ModelParametersStore.defaultParameters)
        val replacement = names.mapIndexed { order, name ->
            val matches = group.filter { it.modelName == name }
            require(matches.size <= 1) { "Ambiguous configured profile identity for model: $name" }
            val previous = matches.singleOrNull()
            val id = previous?.id ?: if (catalog.none { it.id == name }) name else UUID.randomUUID().toString()
            CanonicalModelProfile(
                id = id,
                displayName = previous?.displayName ?: name,
                provider = "openai-compatible", modelName = name, runtime = "remote",
                sourceJson = JsonObject(json.parseToJsonElement(sourceJson(previous ?: active, endpointId, normalizedEndpoint, order, previous == null, selectedProtocol)).jsonObject + mapOf("credentialRef" to (credentialReference?.let(::JsonPrimitive) ?: kotlinx.serialization.json.JsonNull), "credentialRequired" to JsonPrimitive(credentialReference != null))).toString(),
                parametersJson = previous?.parametersJson ?: defaults,
                selected = name == selected, enabled = true, revision = previous?.revision ?: 0,
                createdAtMs = previous?.createdAtMs ?: now, updatedAtMs = now, deletedAtMs = null,
            )
        }
        val retained = if (replacement.isEmpty() && untouched.none { it.selected }) {
            val fallback = untouched.firstOrNull { it.enabled }
            untouched.map { if (it.id == fallback?.id) it.copy(selected = true) else it }
        } else untouched
        runtime.replaceModelProfiles("openai-compatible", "remote", retained + replacement, expectedRevisions, now)
    }

    fun clearAll() {
        val catalog = runtime.listModelProfiles().filter {
            it.provider == "openai-compatible" && it.runtime == "remote" && it.deletedAtMs == null
        }
        runtime.replaceModelProfiles("openai-compatible", "remote", emptyList(),
            catalog.associate { it.id to it.revision }, System.currentTimeMillis())
    }

    fun validateInput(endpoint: String, models: String) {
        val names = parseModelNames(models)
        validateInput(endpoint.trim(), names)
    }

    private fun validateInput(endpoint: String, names: List<String>) {
        require(endpoint.isEmpty() == names.isEmpty()) { "Endpoint and model names must be configured together" }
        require(endpoint.isEmpty() || RemoteModelEndpointURL.isValid(endpoint)) { "Invalid remote model endpoint URL" }
    }

    internal fun mergeAddedModelProfiles(
        catalog: List<CanonicalModelProfile>,
        normalizedEndpoint: String,
        names: List<String>,
        wireProtocol: String?,
        credentialReference: String?,
        now: Long,
    ): List<CanonicalModelProfile> = mergeAddedRemoteModelProfiles(
        catalog = catalog,
        normalizedEndpoint = normalizedEndpoint,
        names = names,
        wireProtocol = wireProtocol,
        credentialReference = credentialReference,
        now = now,
    )

    private fun source(profile: CanonicalModelProfile): Source = parseRemoteModelSource(profile)

    private fun groupKey(source: Source): String = source.endpointID?.takeIf { it.isNotBlank() } ?: source.baseURL

    private fun sourceJson(
        template: CanonicalModelProfile?,
        endpointId: String,
        endpoint: String,
        order: Int,
        isNewModel: Boolean,
        wireProtocol: String,
        defaultEndpointOrder: Int = 0,
    ): String = buildRemoteModelSourceJson(template, endpointId, endpoint, order, isNewModel, wireProtocol, defaultEndpointOrder)

    @Serializable
    internal data class Source(
        val schemaVersion: Int = 1,
        val baseURL: String,
        val endpointID: String? = null,
        val endpointOrder: Int = 0,
        val modelOrder: Int = 0,
        val wireProtocol: String = "chat_completions",
    )
}

private val remoteModelConfigJson = Json { encodeDefaults = true; ignoreUnknownKeys = true }

private fun parseRemoteModelSource(profile: CanonicalModelProfile): AndroidModelConfiguration.Source =
    remoteModelConfigJson.decodeFromString<AndroidModelConfiguration.Source>(profile.sourceJson).also {
        require(it.schemaVersion in 1..2 && it.baseURL.isNotBlank()) { "Unsupported API model source" }
    }

private fun buildRemoteModelSourceJson(
    template: CanonicalModelProfile?,
    endpointId: String,
    endpoint: String,
    order: Int,
    isNewModel: Boolean,
    wireProtocol: String,
    defaultEndpointOrder: Int = 0,
): String {
    val fields = template?.let { remoteModelConfigJson.parseToJsonElement(it.sourceJson).jsonObject.toMutableMap() } ?: mutableMapOf()
    if (isNewModel) {
        fields.remove("reasoning")
        // Execution support belongs to the exact model, never its sibling.
        fields.remove("executionCapabilities")
        fields.remove("reasoningSummarySupported")
        fields["priority"] = JsonPrimitive(50)
        fields["capabilities"] = JsonArray(listOf(JsonPrimitive("chat")))
    }
    fields["schemaVersion"] = JsonPrimitive(2)
    fields["endpointID"] = JsonPrimitive(endpointId)
    fields["endpointName"] = fields["endpointName"] ?: JsonPrimitive(URI(endpoint).host ?: endpoint)
    fields["baseURL"] = JsonPrimitive(endpoint)
    fields["endpointEnabled"] = fields["endpointEnabled"] ?: JsonPrimitive(true)
    fields["endpointOrder"] = fields["endpointOrder"] ?: JsonPrimitive(defaultEndpointOrder)
    fields["modelEnabled"] = JsonPrimitive(true)
    fields["modelOrder"] = JsonPrimitive(order)
    fields["priority"] = fields["priority"] ?: JsonPrimitive(50)
    fields["capabilities"] = fields["capabilities"] ?: JsonArray(listOf(JsonPrimitive("chat")))
    fields["wireProtocol"] = JsonPrimitive(wireProtocol)
    return JsonObject(fields).toString()
}

internal fun mergeAddedRemoteModelProfiles(
    catalog: List<CanonicalModelProfile>,
    normalizedEndpoint: String,
    names: List<String>,
    wireProtocol: String?,
    credentialReference: String?,
    now: Long,
): List<CanonicalModelProfile> {
    val matchingGroup = catalog.filter { parseRemoteModelSource(it).baseURL == normalizedEndpoint }
    val template = matchingGroup.firstOrNull { it.enabled } ?: matchingGroup.firstOrNull()
    val templateSource = template?.let(::parseRemoteModelSource)
    val selectedProtocol = wireProtocol ?: templateSource?.wireProtocol ?: "chat_completions"
    val endpointId = templateSource?.endpointID?.takeIf { it.isNotBlank() } ?: UUID.randomUUID().toString()
    val endpointOrder = templateSource?.endpointOrder
        ?: ((catalog.maxOfOrNull { parseRemoteModelSource(it).endpointOrder } ?: -1) + 1)
    var nextModelOrder = (matchingGroup.maxOfOrNull { parseRemoteModelSource(it).modelOrder } ?: -1) + 1
    val defaults = remoteModelConfigJson.encodeToString(ModelParametersStore.defaultParameters)
    val updatedIds = mutableSetOf<String>()
    val allocatedIds = mutableSetOf<String>()
    val selectedName = names.firstOrNull()
    val upserted = names.map { name ->
        val matches = matchingGroup.filter { it.modelName == name }
        require(matches.size <= 1) { "Ambiguous configured profile identity for model: $name" }
        val previous = matches.singleOrNull()
        if (previous != null) updatedIds += previous.id
        val id = previous?.id ?: if (catalog.none { it.id == name } && name !in allocatedIds) name else UUID.randomUUID().toString()
        allocatedIds += id
        val order = previous?.let { parseRemoteModelSource(it).modelOrder } ?: nextModelOrder++
        val baseSource = buildRemoteModelSourceJson(
            template = previous ?: template,
            endpointId = endpointId,
            endpoint = normalizedEndpoint,
            order = order,
            isNewModel = previous == null,
            wireProtocol = selectedProtocol,
            defaultEndpointOrder = endpointOrder,
        )
        val sourceWithCredential = JsonObject(
            remoteModelConfigJson.parseToJsonElement(baseSource).jsonObject + mapOf(
                "credentialRef" to (credentialReference?.let(::JsonPrimitive) ?: kotlinx.serialization.json.JsonNull),
                "credentialRequired" to JsonPrimitive(credentialReference != null),
            )
        ).toString()
        CanonicalModelProfile(
            id = id,
            displayName = previous?.displayName ?: name,
            provider = "openai-compatible",
            modelName = name,
            runtime = "remote",
            sourceJson = sourceWithCredential,
            parametersJson = previous?.parametersJson ?: defaults,
            selected = name == selectedName,
            enabled = true,
            revision = previous?.revision ?: 0,
            createdAtMs = previous?.createdAtMs ?: now,
            updatedAtMs = now,
            deletedAtMs = null,
        )
    }
    val retained = catalog.filter { it.id !in updatedIds }.map { it.copy(selected = false) }
    return retained + upserted
}

internal fun parseModelNames(models: String): List<String> =
    models.split(',', ';', '\n').map { it.trim() }.filter { it.isNotEmpty() }.distinct()
