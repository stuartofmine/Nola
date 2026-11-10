package com.nola.core

import com.nola.data.local.UiSurfaceRevisionRef
import kotlinx.serialization.json.*

/** Queue contents remain owned by Core; this is only the human timeline summary. */
internal fun queuedUiInputPresentation(submission: JsonObject, conversationId: String, commandId: String): Pair<String, String> {
    require(submission.getValue("type").jsonPrimitive.content == "user_input" &&
        submission.getValue("submission_id").jsonPrimitive.content == commandId)
    val item = submission.getValue("items").jsonArray.single().jsonObject
    require(item.getValue("type").jsonPrimitive.content == "ui_action")
    val id = item.getValue("id").jsonPrimitive.content
    require(id == "ui:$commandId:user")
    val action = item.getValue("action").jsonObject
    val surface = Json.decodeFromString(UiSurfaceRevisionRef.serializer(), action.getValue("surface").toString())
    require(surface.identity.owner.conversation_id == conversationId && surface.revision > 0)
    val summary = action.getValue("summary").jsonPrimitive.content
    require(summary.isNotBlank())
    return id to summary
}
