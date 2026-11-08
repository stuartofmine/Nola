package com.nola.data.local

import kotlinx.serialization.json.*

/** Private transport serialization only. It is never an authorization decision
 * and never classifies generic renderer commands. Unknown input stays global. */
internal fun expertHostOperationScope(command: JsonObject): ApplicationCommandScope = runCatching {
    require(command["type"] == JsonPrimitive("native_expert_host_operation"))
    val operation = command.getValue("operation").jsonObject
    val input = operation.getValue("input").jsonObject
    fun id(value: JsonElement?): String {
        val text = checkNotNull(value).jsonPrimitive
        require(text.isString && text.content.isNotBlank() && text.content.length <= 512 &&
            text.content.trim() == text.content && text.content.none(Char::isISOControl))
        return text.content
    }
    val owners = when (operation.getValue("kind").jsonPrimitive.content) {
        "admit" -> setOf(id(input["conversation_id"])) + input.getValue("selected_participants").jsonArray.map {
            id(it.jsonObject["context_conversation_id"])
        }.also { require(it.isNotEmpty()) }
        "prepare_edited_resend" -> setOf(id(input["source_conversation_id"]), id(input["target_conversation_id"]))
        else -> error("Not a private mutating operation")
    }
    ApplicationCommandScope.Owners(owners)
}.getOrDefault(ApplicationCommandScope.Global)
