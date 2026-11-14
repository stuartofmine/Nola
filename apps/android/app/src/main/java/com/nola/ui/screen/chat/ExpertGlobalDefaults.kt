package com.nola.ui.screen.chat

import com.nola.data.local.CanonicalModelProfile
import kotlinx.serialization.json.*

/** Same ordered model facts as Core expert preview; a parent conversation is never an input. */
internal fun expertDefaultModelResolutionCommand(profiles: List<CanonicalModelProfile>, preferenceJson: String?): JsonObject = buildJsonObject {
    put("type", "resolve_chat_model")
    put("intent", buildJsonObject { put("kind", "inherit") })
    put("default_value_json", preferenceJson?.let(::JsonPrimitive) ?: JsonNull)
    put("candidates", JsonArray(profiles.filter { it.deletedAtMs == null }.map { profile -> buildJsonObject {
        put("profile_id", profile.id); put("allowed_for_chat", true)
        put("availability", if (profile.enabled) "available" else "disabled")
    } }))
}
internal fun resolvedExpertDefaultProfileId(value: JsonElement): String? {
    val result = value as? JsonObject ?: return null
    return if (result["kind"] == JsonPrimitive("resolved"))
        (result["profile_id"] as? JsonPrimitive)?.takeIf { it.isString }?.content?.takeIf { it.isNotBlank() } else null
}
