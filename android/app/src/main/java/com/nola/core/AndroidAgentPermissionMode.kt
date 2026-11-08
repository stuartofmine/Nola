package com.nola.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Canonical product modes shared with the CLI permission panel. */
enum class AndroidAgentPermissionMode(val wireValue: String) {
    Cautious("cautious"),
    AllowAll("allow_all");

    companion object {
        fun fromWireValue(value: String?): AndroidAgentPermissionMode = when (value) {
            null, Cautious.wireValue -> Cautious
            AllowAll.wireValue -> AllowAll
            else -> throw IllegalArgumentException("Permission profile must be cautious or allow_all")
        }
    }
}

internal fun AndroidAgentPermissionMode.corePermissionRule(): JsonObject? = when (this) {
    AndroidAgentPermissionMode.Cautious -> permissionRule(
        decision = "allow",
        source = "product_default",
    )
    AndroidAgentPermissionMode.AllowAll -> permissionRule(
        decision = "allow",
        source = "invocation_policy",
    )
}

internal fun permissionRule(
    capability: String = "*",
    resource: String = "*",
    decision: String,
    source: String,
): JsonObject = buildJsonObject {
    put("capability_pattern", capability)
    put("resource_pattern", resource)
    put("decision", decision)
    put("source", source)
}

internal fun androidAgentPermissionRules(toolPlan: AgentCoreAndroidToolPlan): JsonObject =
    buildJsonObject {
        put(
            "rules",
            buildJsonArray {
                toolPlan.permissionMode.corePermissionRule()?.let(::add)
                toolPlan.allowedTools
                    .filter { it.permission != "none" && it.authorization in setOf("ask", "granted") }
                    .forEach { tool ->
                        add(permissionRule(
                            capability = tool.capability,
                            decision = if (tool.authorization == "granted") "allow" else "ask",
                            source = "host",
                        ))
                    }
                toolPlan.scopedGrants
                    .filter { it.scope.trim().isNotEmpty() }
                    .forEach { grant ->
                        add(permissionRule(
                            capability = grant.capability,
                            resource = grant.scope.trim(),
                            decision = "allow",
                            source = "host",
                        ))
                    }
            },
        )
    }
