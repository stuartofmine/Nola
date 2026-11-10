// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.TodayFactsSnapshot
import kotlinx.serialization.json.*
import javax.inject.Inject

/** The only Today read is the Application's pinned snapshot. No local Run
 * discovery, ownership joins, pending-state inference, or decision commands. */
internal class TodayPendingResponseReader(private val query: (JsonObject) -> JsonElement) {
    @Inject constructor(runtime: AndroidApplicationRuntime) : this(runtime::executeQuery)

    fun read(): TodayFactsSnapshot = Json.decodeFromJsonElement(query(buildJsonObject {
        put("type", "load_today_facts")
        put("input", buildJsonObject {
            put("conversation_offset", 0)
            put("conversation_limit", 50)
            put("queued_limit", 10)
        })
    }))
}
