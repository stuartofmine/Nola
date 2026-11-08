package com.nola.data.repository

import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.serialization.json.*

data class GoalOperationObservation(val commandSha256: String, val goalRevision: ULong, val rejection: String?)
data class GoalControlObservation(val goalId: String?, val controlToken: String?, val revision: ULong,
    val appliedOperations: Map<String, GoalOperationObservation>,
    val canPause: Boolean, val canResume: Boolean, val canCancel: Boolean)
data class ConversationProgressItem(val id: String, val text: String, val status: String)
data class ConversationProgress(val conversationId: String, val objective: String?, val status: String?,
    val requirements: List<ConversationProgressItem>, val todos: List<ConversationProgressItem>,
    val goalControl: GoalControlObservation? = null)

class ConversationProgressRepository(private val runtime: AndroidApplicationRuntime) {
    fun load(owner: String): ConversationProgress {
        val projection = runtime.executeQuery(buildJsonObject { put("type", "load_session_projection"); put("conversation_id", owner) })
        val plan = runtime.executeQuery(buildJsonObject {
            put("type", "conversation_plan")
            put("command", buildJsonObject { put("type", "get"); put("conversation_id", owner) })
        })
        val observation = runtime.executeQuery(buildJsonObject {
            put("type", "load_goal_control_observation"); put("conversation_id", owner)
        })
        return decode(owner, withLocalControlObservation(projection, observation), plan)
    }

    companion object {
        internal fun withLocalControlObservation(projection: JsonElement, observation: JsonElement): JsonElement {
            val session = projection as? JsonObject ?: return projection
            val goalProjection = session["goal_projection"] as? JsonObject ?: return projection
            val active = goalProjection["active"] as? JsonObject
            val observed = observation as? JsonObject
            require((active == null) == (observed == null)) { "Goal changed while loading progress; refresh required" }
            if (active != null && observed != null) {
                require(active["id"] == observed["goal_id"] && active["status"] == observed["status"]) {
                    "Goal changed while loading progress; refresh required"
                }
            }
            val enriched = JsonObject(goalProjection + buildJsonObject {
                put("control_token", observed?.getValue("control_token") ?: JsonNull)
                for (key in listOf("can_pause", "can_resume", "can_cancel"))
                    put(key, observed?.getValue(key) ?: JsonPrimitive(false))
            })
            return JsonObject(session + ("goal_projection" to enriched))
        }

        internal fun decodeRemote(owner: String, hostId: String, response: JsonObject): ConversationProgress {
            require(response.keys == setOf("source_host_id", "conversation_id", "goal_projection", "plan")) { "Invalid remote progress response" }
            require(response["source_host_id"]?.jsonPrimitive?.isString == true && response["source_host_id"]?.jsonPrimitive?.content == hostId && hostId.isNotBlank()) { "Progress Host mismatch" }
            require(response["conversation_id"]?.jsonPrimitive?.isString == true && response["conversation_id"]?.jsonPrimitive?.content == owner && owner.isNotBlank()) { "Progress owner mismatch" }
            val projection = buildJsonObject {
                put("conversation_id", owner); put("goal_projection", response.getValue("goal_projection"))
            }
            return decode(owner, projection, response.getValue("plan"), requireRemoteControl = true)
        }

        internal fun decode(owner: String, projection: JsonElement, plan: JsonElement, requireRemoteControl: Boolean = false): ConversationProgress {
            val session = projection.takeUnless { it is JsonNull }?.jsonObject
            val savedPlan = plan.takeUnless { it is JsonNull }?.jsonObject
            require(session == null || session["conversation_id"]?.jsonPrimitive?.content == owner) { "Progress owner mismatch" }
            require(savedPlan == null || savedPlan["conversation_id"]?.jsonPrimitive?.content == owner) { "Plan owner mismatch" }
            val goalProjection = session?.get("goal_projection")?.takeUnless { it is JsonNull }?.jsonObject
            val goal = goalProjection?.get("active")?.takeUnless { it is JsonNull }?.jsonObject
            fun text(value: JsonElement, label: String): String {
                val primitive = value as? JsonPrimitive ?: error("Invalid $label")
                require(primitive.isString) { "Invalid $label" }
                return primitive.content
            }
            fun identity(value: JsonElement, label: String): String = text(value, label).also {
                require(it.isNotBlank() && it.toByteArray().size <= 512 && it.none { character -> character.isISOControl() }) { "Invalid $label" }
            }
            fun revision(value: JsonElement): ULong {
                val primitive = value as? JsonPrimitive ?: error("Invalid Goal revision")
                require(!primitive.isString)
                return primitive.content.toULongOrNull()?.also { require(it.toString() == primitive.content) }
                    ?: error("Invalid Goal revision")
            }
            val control = goalProjection?.let { value ->
                val id = goal?.let { identity(it.getValue("id"), "Goal identity") }
                val token = value["control_token"]?.takeUnless { it is JsonNull }?.let { identity(it, "Goal control token") }
                require(!requireRemoteControl || ("control_token" in value && (id == null) == (token == null))) { "Missing authoritative Goal control token" }
                require(token == null || id != null)
                goal?.let { require(text(it.getValue("status"), "Goal status") in setOf("running","paused","needs_user","completed","failed","cancelled")) }
                val generation = revision(value.getValue("control_generation"))
                val observedRevision = revision(value.getValue("revision"))
                require(generation <= observedRevision && (id == null || generation > 0uL))
                val operations = value.getValue("applied_operations").jsonObject.mapValues { (operationId, entry) ->
                    require(operationId.isNotBlank())
                    val operation = entry.jsonObject
                    require(operation.keys == setOf("command_sha256", "goal_revision", "rejection"))
                    val digest = text(operation.getValue("command_sha256"), "Goal operation digest")
                    require(digest.matches(Regex("[0-9a-f]{64}")))
                    val appliedRevision = revision(operation.getValue("goal_revision"))
                    require(appliedRevision <= observedRevision)
                    GoalOperationObservation(digest, appliedRevision, operation.getValue("rejection").takeUnless { it is JsonNull }?.let { text(it, "Goal rejection") })
                }
                fun capability(key: String): Boolean {
                    val flag = value.getValue(key) as? JsonPrimitive ?: error("Invalid Goal control capability")
                    require(!flag.isString) { "Invalid Goal control capability" }
                    return flag.booleanOrNull ?: error("Invalid Goal control capability")
                }
                GoalControlObservation(id, token, observedRevision, operations,
                    capability("can_pause"), capability("can_resume"), capability("can_cancel"))
            }
            fun item(value: JsonElement, textKey: String): ConversationProgressItem {
                val obj = value.jsonObject
                return ConversationProgressItem(obj.getValue("id").jsonPrimitive.content,
                    obj.getValue(textKey).jsonPrimitive.content, obj.getValue("status").jsonPrimitive.content)
            }
            return ConversationProgress(owner, goal?.get("objective")?.jsonPrimitive?.content,
                goal?.get("status")?.jsonPrimitive?.content,
                goal?.get("requirements")?.jsonObject?.values?.map { item(it, "description") }.orEmpty(),
                savedPlan?.get("todos")?.jsonArray?.map { item(it, "content") }.orEmpty(), control)
        }
    }
}
