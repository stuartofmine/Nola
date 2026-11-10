package com.nola.ui.screen.chat

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Maps Core's retained history back to the persisted messages used by Android
 * agent runtimes. The latest user turn is always retained and placed last so
 * callers can remove that single duplicate before passing `input` separately.
 */
internal fun resolveCorePreparedIndices(
    activeMessageIds: List<String>,
    preparedMessages: List<JsonElement>,
    latestUserId: String?,
): List<Int> {
    val indexById = activeMessageIds.withIndex().associate { it.value to it.index }
    val retained = mutableListOf<Int>()
    for (element in preparedMessages) {
        val id = runCatching {
            element.jsonObject["id"]?.jsonPrimitive?.content
        }.getOrNull() ?: return emptyList()
        val index = indexById[id] ?: return emptyList()
        if (index !in retained) retained += index
    }

    latestUserId?.let(indexById::get)?.let { latestIndex ->
        retained.remove(latestIndex)
        retained += latestIndex
    }
    return retained
}
