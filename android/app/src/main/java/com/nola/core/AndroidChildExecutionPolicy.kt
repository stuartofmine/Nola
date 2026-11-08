package com.nola.core

import kotlinx.serialization.json.*

internal data class AndroidChildExecutionPolicy(
    val permissionMode: AndroidAgentPermissionMode,
    val resultEgressAuthorized: Boolean,
)

/** Input is Core's exact admitted child configuration, never today's composer settings. */
internal fun androidChildExecutionPolicy(config: JsonObject): AndroidChildExecutionPolicy {
    val rules = config.getValue("permission_rules").jsonObject.getValue("rules").jsonArray
    val modes = AndroidAgentPermissionMode.entries.filter { mode ->
        val rule = mode.corePermissionRule()
        rule != null && rules.any { it == rule }
    }
    require(modes.size == 1) { "Child invocation permission policy is missing or ambiguous" }
    var egress = false
    for (tool in config.getValue("tools").jsonArray) {
        val raw = tool.jsonObject["x_nola"]?.jsonObject?.get("requires_result_egress") ?: continue
        val flag = raw as? JsonPrimitive
        require(flag != null && !flag.isString && flag.booleanOrNull != null) {
            "Invalid admitted child result-egress fact"
        }
        if (flag.boolean) egress = true
    }
    return AndroidChildExecutionPolicy(modes.single(), egress)
}
