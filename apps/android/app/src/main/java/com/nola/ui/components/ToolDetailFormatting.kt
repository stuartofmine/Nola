package com.nola.ui.components

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

private val toolDetailJson = Json { prettyPrint = true }

/** Presentation only: preserve plain output and never rewrite the stored tool payload. */
internal fun formattedToolDetail(value: String): String {
    val parsed = runCatching { Json.parseToJsonElement(value) }.getOrNull()
    return if (parsed is JsonObject || parsed is JsonArray) {
        toolDetailJson.encodeToString(JsonElement.serializer(), parsed)
    } else value
}

internal data class ToolDetailExcerpt(val text: String, val truncated: Boolean)

internal fun toolDetailExcerpt(value: String): ToolDetailExcerpt {
    val clipped = value.lineSequence().take(17).toList()
    val preview = clipped.take(16).joinToString("\n").take(2000)
    val truncated = clipped.size > 16 || preview.length < value.length
    return ToolDetailExcerpt(if (truncated) "$preview…" else preview, truncated)
}
