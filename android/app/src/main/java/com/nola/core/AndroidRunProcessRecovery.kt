package com.nola.core

import kotlinx.serialization.json.*

/** Transport outcome only; Core owns the Run and its committed business result. */
internal sealed interface AndroidRunRecoveryOutcome {
    data class Active(val projection: JsonObject) : AndroidRunRecoveryOutcome
    data class Sealed(val seal: AndroidCanonicalRunTerminalSeal) : AndroidRunRecoveryOutcome
}

internal fun sameAndroidFrozenRunAuthority(original: JsonObject, current: JsonObject): Boolean =
    listOf("conversation_id", "run_id", "assistant_message_id", "invocation_config_digest",
        "host_invocation", "context_budget", "core_session_id", "remote_tool_plan").all { original[it] == current[it] }

/** A concurrent seal is acceptable only for the original immutable invocation. */
internal fun loadAndroidRecoveryTerminalIfSealed(
    descriptor: AndroidCanonicalTurnDescriptor,
    frozen: JsonObject,
    current: JsonObject,
    execute: (JsonObject) -> JsonElement,
): AndroidCanonicalRunTerminalSeal? {
    if (current["status"]?.jsonPrimitive?.contentOrNull != "sealed") return null
    require(sameAndroidFrozenRunAuthority(frozen, current)) { "Sealed recovery invocation changed authority" }
    val coreSession = checkNotNull(current["core_session_id"]?.jsonPrimitive?.contentOrNull)
    return checkNotNull(loadAndroidCanonicalTerminalSeal(descriptor, coreSession, execute)) {
        "Sealed recovery has no exact committed terminal seal"
    }
}

/** Host supplies OS facts, never parses worker ownership or invents a retry. */
internal fun reconcileAndroidObservedRun(
    descriptor: AndroidCanonicalTurnDescriptor,
    frozen: JsonObject,
    execute: (JsonObject) -> JsonElement,
    observe: (Int) -> JsonObject,
    nowMs: () -> Long,
): Boolean {
    val plan = execute(buildJsonObject {
        put("type", "inspect_run_process_owners")
        put("conversation_id", descriptor.conversationId)
        put("expected_run_id", descriptor.submissionId)
    }).jsonObject
    require(plan["conversation_id"] == JsonPrimitive(descriptor.conversationId) &&
        plan["run_id"] == JsonPrimitive(descriptor.submissionId) &&
        plan["assistant_message_id"] == JsonPrimitive(descriptor.assistantMessageId) &&
        plan["invocation_config_digest"] == frozen["invocation_config_digest"] &&
        plan["host_invocation"] == frozen["host_invocation"]) { "Run process observation changed owner" }
    // Independent session leases require their own trusted owner observation.
    if (plan.getValue("has_session_lease").jsonPrimitive.boolean) return false
    val owners = plan.getValue("process_owners").jsonArray
    val deliveries = plan.getValue("deliveries").jsonArray
    val observations = JsonArray(owners.map { value ->
        val owner = value.jsonObject
        val pid = owner.getValue("pid").jsonPrimitive.int
        require(pid > 0) { "Invalid observed worker PID" }
        buildJsonObject { put("owner", owner); put("observation", observe(pid)) }
    })
    val terminated = execute(buildJsonObject {
        put("type", "validate_run_process_observations")
        put("plan", plan); put("observations", observations)
    }).jsonPrimitive.boolean
    if (!terminated) return false
    val now = nowMs()
    execute(buildJsonObject {
        put("type", "recover_observed_orphaned_run")
        put("conversation_id", descriptor.conversationId); put("expected_run_id", descriptor.submissionId)
        put("host_invocation", frozen.getValue("host_invocation"))
        put("observed_session_lease", JsonNull); put("observed_deliveries", deliveries)
        put("interrupted_at", java.time.Instant.ofEpochMilli(now).toString()); put("interrupted_at_ms", now)
    })
    return true
}
