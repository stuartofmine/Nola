package com.nola.data.repository

import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.serialization.json.*

data class ModelAccountSummary(val id: String, val name: String, val source: String, val state: String)
data class ModelConnectionSummary(val id: String, val name: String, val address: String,
    val accountId: String?, val source: String, val enabled: Boolean, val models: List<String>)
data class ModelConnectionInventory(val connections: List<ModelConnectionSummary>, val accounts: List<ModelAccountSummary>)

/** Only public Core projections cross this boundary. Credential references are not UI fields. */
class ModelConnectionRepository(private val runtime: AndroidApplicationRuntime) {
    fun load(): ModelConnectionInventory {
        fun read(type: String) = runtime.execute(buildJsonObject {
            put("type", "model"); put("command", buildJsonObject { put("type", type) })
        }).jsonArray
        val connections = read("list_connections").map { snapshot ->
            val document = snapshot.jsonObject.getValue("document").jsonObject
            require(document.getValue("format_version").jsonPrimitive.int == 1) { "Unsupported model connection format" }
            val value = document.getValue("connection").jsonObject
            ModelConnectionSummary(value.getValue("id").jsonPrimitive.content,
                value.getValue("display_name").jsonPrimitive.content, value.getValue("base_url").jsonPrimitive.content,
                value["account_id"]?.jsonPrimitive?.contentOrNull, value.getValue("source").jsonPrimitive.content,
                value.getValue("enabled").jsonPrimitive.boolean,
                (document["catalog"] as? JsonObject)?.get("models")?.jsonArray.orEmpty().map {
                    it.jsonObject.getValue("display_name").jsonPrimitive.content
                })
        }
        val accounts = read("list_accounts").map { snapshot ->
            val value = snapshot.jsonObject.getValue("account").jsonObject
            ModelAccountSummary(value.getValue("id").jsonPrimitive.content, value.getValue("display_name").jsonPrimitive.content,
                value.getValue("source").jsonPrimitive.content, value.getValue("state").jsonPrimitive.content)
        }
        return ModelConnectionInventory(connections, accounts)
    }
}
