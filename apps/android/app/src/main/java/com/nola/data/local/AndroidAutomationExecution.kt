package com.nola.data.local

import com.nola.core.AndroidAgentPermissionMode
import kotlinx.serialization.json.*

/** Definition authority and occurrence ownership; never a client success flag. */
data class AndroidAutomationSpec(val prompt: String, val hostId: String,
    val permissionProfile: String, val contextRefs: List<Pair<String, String>>, val skillRefs: List<String>) {
    init {
        require(prompt.isNotBlank() && hostId.isNotBlank())
        AndroidAgentPermissionMode.fromWireValue(permissionProfile)
        require(contextRefs.all { it.first in setOf("personal_data", "web_source") && it.second.isNotBlank() })
        require(skillRefs.all(String::isNotBlank))
    }
    fun toJson(): JsonObject = buildJsonObject {
        put("prompt", prompt)
        put("execution_scope", buildJsonObject { put("kind", "device_local"); put("host_id", hostId) })
        put("permission_profile", permissionProfile)
        put("model_selection", "use_current_default_at_admission")
        put("context_refs", JsonArray(contextRefs.map { (kind, id) -> buildJsonObject { put("kind", kind); put("id", id) } }))
        put("skill_refs", JsonArray(skillRefs.map(::JsonPrimitive)))
    }
    companion object {
        fun decode(value: JsonObject): AndroidAutomationSpec {
            val scope = value.getValue("execution_scope").jsonObject
            require(scope.getValue("kind").jsonPrimitive.content == "device_local")
            require(value.getValue("model_selection").jsonPrimitive.content == "use_current_default_at_admission")
            return AndroidAutomationSpec(value.getValue("prompt").jsonPrimitive.content,
                scope.getValue("host_id").jsonPrimitive.content,
                value.getValue("permission_profile").jsonPrimitive.content,
                value.getValue("context_refs").jsonArray.map { it.jsonObject.let { ref ->
                    ref.getValue("kind").jsonPrimitive.content to ref.getValue("id").jsonPrimitive.content } },
                value.getValue("skill_refs").jsonArray.map { it.jsonPrimitive.content })
        }
    }
}

data class AndroidAutomationDefinition(val id: String, val revision: Long, val spec: AndroidAutomationSpec,
    val metadata: JsonObject)
data class AndroidAutomationLease(val occurrenceId: String, val definitionRevision: Long,
    val workerId: String, val leaseToken: String) {
    init {
        require(definitionRevision > 0)
        require(occurrenceId.isNotBlank() && workerId.isNotBlank() && leaseToken.isNotBlank())
    }
    fun toJson() = buildJsonObject {
        put("occurrence_id", occurrenceId); put("definition_revision", definitionRevision)
        put("worker_id", workerId); put("lease_token", leaseToken)
    }
}

internal fun decodeAndroidAutomationDefinition(value: JsonObject): AndroidAutomationDefinition {
    val execution = value.getValue("execution_spec").jsonObject
    require(execution.getValue("kind").jsonPrimitive.content == "agent")
    val revision = value.getValue("revision").jsonPrimitive.long
    require(revision > 0)
    return AndroidAutomationDefinition(value.getValue("id").jsonPrimitive.content, revision,
        AndroidAutomationSpec.decode(execution.getValue("spec").jsonObject),
        Json.parseToJsonElement(value.getValue("metadata_json").jsonPrimitive.content).jsonObject)
}
