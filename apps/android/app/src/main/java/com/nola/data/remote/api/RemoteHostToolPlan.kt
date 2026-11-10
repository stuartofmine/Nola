// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import kotlinx.serialization.json.*

private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
private fun JsonObject.integer(key: String): Long? = (this[key] as? JsonPrimitive)?.takeUnless { it.isString }?.longOrNull
private fun validIdentity(value: String): Boolean = value.isNotBlank() && value.toByteArray(Charsets.UTF_8).size <= 512 &&
    value.none { it.code <= 0x1f || it.code in 0x7f..0x9f }

/** Only the authenticated execution Host can issue this reference. No local platform inference. */
fun validatedHostToolPlanReference(response: JsonObject, hostId: String, clientId: String,
    sessionId: String): JsonObject {
    require(listOf(hostId, clientId, sessionId).all(::validIdentity))
    require(response.text("source_host_id") == hostId)
    val plan = response["result"] as? JsonObject ?: error("Missing execution Host tool plan")
    require(plan.keys == setOf("schema_version", "reference", "issued_at_ms", "expires_at_ms", "tools"))
    require(plan.integer("schema_version") == 1L)
    val ref = plan["reference"] as? JsonObject ?: error("Missing Host catalog reference")
    require(ref.keys == setOf("manifest_id", "host_id", "client_id", "session_id", "catalog_revision"))
    require(ref.text("host_id") == hostId && ref.text("client_id") == clientId && ref.text("session_id") == sessionId)
    require(ref.text("manifest_id")?.let(::validIdentity) == true)
    require(ref.text("catalog_revision")?.matches(Regex("[0-9a-f]{64}")) == true)
    val issued = plan.integer("issued_at_ms") ?: error("Missing catalog issuance")
    val expires = plan.integer("expires_at_ms") ?: error("Missing catalog expiry")
    // Freshness is checked by the issuing Host at admission, using its own clock.
    require(issued > 0 && expires > issued && expires <= 9_007_199_254_740_991L && expires - issued <= 300_000) { "Invalid Host tool plan interval: issued=$issued expires=$expires" }
    val tools = plan["tools"] as? JsonArray ?: error("Missing Host tools")
    val seen = mutableSetOf<String>()
    tools.forEach { item ->
        val tool = item as? JsonObject ?: error("Invalid Host tool")
        require(tool.keys == setOf("tool_id", "route", "schema"))
        val id = tool.text("tool_id") ?: error("Missing Host tool identity")
        require(validIdentity(id) && seen.add(id) && tool.text("route") == "agent_host")
        val schema = tool["schema"] as? JsonObject ?: error("Missing Host tool schema")
        require(schema.text("type") == "function")
        val function = schema["function"] as? JsonObject ?: error("Missing Host function schema")
        require(function.text("name") == id && function["parameters"] is JsonObject)
    }
    return ref
}
