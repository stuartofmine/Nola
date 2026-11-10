// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Mirrors Core TaskHandle state and strictly reduces authoritative TaskUpdated events.
// - Boundary: Ordinary tool traces are never promoted to durable tasks by this Android projection.

package com.nola.core

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

enum class AgentCoreTaskKind(val wireValue: String) {
    Agent("agent"),
    Tool("tool"),
    Skill("skill"),
    DataRefresh("data_refresh");

    companion object {
        fun fromWireValue(value: String?): AgentCoreTaskKind? = entries.firstOrNull {
            it.wireValue == value
        }
    }
}

enum class AgentCoreTaskState(val wireValue: String) {
    Queued("queued"),
    Running("running"),
    Waiting("waiting"),
    Suspended("suspended"),
    RetryWaiting("retry_waiting"),
    Completed("completed"),
    Sealed("sealed"),
    Failed("failed"),
    Cancelled("cancelled");

    val isActive: Boolean get() = this in setOf(Queued, Running, Waiting, Suspended, RetryWaiting)
    val isTerminal: Boolean get() = !isActive

    fun canTransitionTo(next: AgentCoreTaskState): Boolean = if (next == Sealed) !isTerminal else when (this) {
        Queued -> true
        Running -> next in setOf(Running, Waiting, Suspended, RetryWaiting, Completed, Failed, Cancelled)
        Waiting -> next in setOf(Waiting, Running, Suspended, RetryWaiting, Completed, Failed, Cancelled)
        Suspended -> next in setOf(Suspended, Running, Failed, Cancelled)
        RetryWaiting -> next in setOf(RetryWaiting, Running, Failed, Cancelled)
        Completed, Sealed, Failed, Cancelled -> false
    }

    companion object {
        fun fromWireValue(value: String?): AgentCoreTaskState? = entries.firstOrNull {
            it.wireValue == value
        }
    }
}

data class AgentCoreTaskDurability(
    val runResult: JsonObject? = null,
    val resultDigest: String? = null,
    val terminalReceiptId: String? = null,
    val workerId: String? = null,
    val ownershipToken: String? = null,
    val leaseUntilEpochMs: Long? = null,
    val lastHeartbeatEpochMs: Long? = null,
    val retryCount: Int = 0,
    val retryAtEpochMs: Long? = null,
    val lastError: String? = null,
    val inputWatermark: Long? = null,
    val completionWatermark: Long? = null,
)

data class AgentCoreTaskLineage(
    val parentTaskId: String? = null,
    val childSessionId: String? = null,
    val parentCallId: String? = null,
    val permissionScopeId: String? = null,
    val cancellationScopeId: String? = null,
)

data class AgentCoreTaskHandle(
    val id: String,
    val ownerSessionId: String,
    val ownerTurnId: String,
    val kind: AgentCoreTaskKind,
    val state: AgentCoreTaskState,
    val progress: Double? = null,
    val currentStep: String? = null,
    val resultRef: String? = null,
    val cancelRequested: Boolean = false,
    val lineage: AgentCoreTaskLineage = AgentCoreTaskLineage(),
    val durability: AgentCoreTaskDurability = AgentCoreTaskDurability(),
) {
    /** Shape/owner check only. Core and Store remain responsible for receipt authenticity. */
    val hasValidResultShape: Boolean get() {
        if (state != AgentCoreTaskState.Sealed) {
            return !(kind == AgentCoreTaskKind.Agent && state == AgentCoreTaskState.Completed) &&
                durability.runResult == null && durability.resultDigest == null
        }
        val result = durability.runResult ?: return false
        val child = lineage.childSessionId ?: return false
        return kind == AgentCoreTaskKind.Agent && child.isNotBlank() &&
            result.stringValue("conversation_id") == child &&
            !result.stringValue("run_id").isNullOrBlank() &&
            result.hasValidRunResultStructure() &&
            durability.resultDigest?.matches(Regex("[0-9a-f]{64}")) == true &&
            !durability.terminalReceiptId.isNullOrBlank()
    }
}

internal fun JsonObject.hasValidRunResultStructure(): Boolean {
    fun identity(value: String?) = value != null && value.isNotEmpty() && value.trim() == value && value.none { it.isISOControl() }
    if (listOf("result_id", "originating_submission_id", "conversation_id", "run_id").any { !identity(stringValue(it)) }) return false
    val input = objectValue("input") ?: return false
    if (input.stringValue("applications_digest")?.matches(Regex("[0-9a-f]{64}")) != true) return false
    if ((input.stringValue("watermark") == "0") != (input.stringValue("applications_digest") ==
            "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd")) return false
    val numbers = listOf(input.stringValue("watermark")) + listOf("decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier").map { stringValue(it) }
    if (numbers.any { it == null || !it.matches(Regex("0|[1-9][0-9]*")) || it.toLongOrNull() == null }) return false
    val value = objectValue("value") ?: return false
    fun text(key: String) = identity(value.stringValue(key))
    fun refs(key: String, nonempty: Boolean = false): Boolean {
        val refs = value[key] as? JsonArray ?: return false
        val strings = refs.map { (it as? JsonPrimitive)?.takeIf { item -> item.isString }?.contentOrNull }
        return (!nonempty || strings.isNotEmpty()) && strings.all(::identity) && strings.distinct().size == strings.size
    }
    fun reason(): Boolean {
        val reason = value.objectValue("reason") ?: return false
        return reason.stringValue("code")?.matches(Regex("[a-z0-9_.]+")) == true && (reason["detail"] == kotlinx.serialization.json.JsonNull ||
            (reason["detail"] as? JsonPrimitive)?.isString == true)
    }
    fun effects(): Boolean {
        if (!refs("observed_effect_refs") || !refs("unknown_effect_refs")) return false
        val observed = value["observed_effect_refs"] as JsonArray
        return (value["unknown_effect_refs"] as JsonArray).none { it in observed }
    }
    return when (value.stringValue("kind")) {
        "answered" -> refs("answer_part_refs", true)
        "action_completed" -> refs("requirement_acceptance_refs", true)
        "delegation_accepted" -> text("task_id") && text("parent_call_id")
        "incomplete" -> reason() && refs("remaining_requirement_refs") && refs("obligation_refs") &&
            (refs("remaining_requirement_refs", true) || refs("obligation_refs", true))
        "blocked" -> reason() && refs("blocking_fact_refs", true) && !value.stringValue("resolution_condition").isNullOrBlank()
        "cancelled" -> text("cancellation_id") && effects()
        "failed" -> text("error_ref") && effects()
        else -> false
    }
}

data class AgentCoreTaskUpdate(
    val sessionId: String,
    val turnId: String,
    val cursor: Long,
    val revision: Long,
    val taskProjectionRevision: Long,
    val task: AgentCoreTaskHandle,
    val guidance: AgentCoreTaskGuidanceQueue? = null,
)

data class AgentCoreTaskGuidanceMessage(
    val sequence: Long,
    val messageId: String,
    val parentSessionId: String,
    val parentTurnId: String,
    val content: String,
    val enqueuedAtEpochMs: Long,
)

data class AgentCoreTaskGuidanceQueue(
    val lastSequence: Long = 0,
    val acknowledgedSequence: Long = 0,
    val pending: List<AgentCoreTaskGuidanceMessage> = emptyList(),
    val acknowledgedMessageIds: List<String> = emptyList(),
)

data class AgentCoreTaskProjection(
    val sessionId: String? = null,
    val revision: Long = 0,
    val order: List<String> = emptyList(),
    val tasks: Map<String, AgentCoreTaskHandle> = emptyMap(),
    val guidance: Map<String, AgentCoreTaskGuidanceQueue> = emptyMap(),
    val lastCursor: Long = 0,
    val lastRevision: Long = 0,
) {
    val orderedTasks: List<AgentCoreTaskHandle> get() = order.mapNotNull(tasks::get)
}

data class AgentCoreTaskReduction(
    val projection: AgentCoreTaskProjection,
    val applied: Boolean,
)

object AgentCoreTaskProjectionReducer {
    fun reduce(
        current: AgentCoreTaskProjection,
        update: AgentCoreTaskUpdate,
    ): AgentCoreTaskReduction {
        val task = update.task
        if (update.sessionId.isBlank() || update.turnId.isBlank() || task.id.isBlank()) {
            return AgentCoreTaskReduction(current, false)
        }
        if (!task.hasValidResultShape || task.ownerSessionId != update.sessionId || task.ownerTurnId != update.turnId) {
            return AgentCoreTaskReduction(current, false)
        }
        if (current.sessionId != null && current.sessionId != update.sessionId) {
            return AgentCoreTaskReduction(current, false)
        }
        if (update.cursor <= current.lastCursor || update.revision < current.lastRevision) {
            return AgentCoreTaskReduction(current, false)
        }
        if (update.taskProjectionRevision <= current.revision) {
            return AgentCoreTaskReduction(current, false)
        }
        if (task.progress?.let { !it.isFinite() || it !in 0.0..1.0 } == true) {
            return AgentCoreTaskReduction(current, false)
        }
        if (task.currentStep?.isBlank() == true || task.resultRef?.isBlank() == true) {
            return AgentCoreTaskReduction(current, false)
        }

        val existing = current.tasks[task.id]
        val guidanceChanged = update.guidance != null && current.guidance[task.id] != update.guidance
        if (existing == null) {
            if (!task.state.isActive) return AgentCoreTaskReduction(current, false)
        } else if (
            existing.ownerSessionId != task.ownerSessionId ||
            existing.ownerTurnId != task.ownerTurnId ||
            existing.kind != task.kind ||
            existing.lineage != task.lineage ||
            !existing.state.canTransitionTo(task.state) ||
            existing.progress != null && task.progress != null && task.progress < existing.progress ||
            existing.cancelRequested && !task.cancelRequested ||
            existing == task && !guidanceChanged
        ) {
            return AgentCoreTaskReduction(current, false)
        }

        val nextOrder = if (existing == null) current.order + task.id else current.order
        return AgentCoreTaskReduction(
            projection = current.copy(
                sessionId = update.sessionId,
                revision = update.taskProjectionRevision,
                order = nextOrder,
                tasks = current.tasks + (task.id to task),
                guidance = update.guidance?.let { current.guidance + (task.id to it) }
                    ?: current.guidance,
                lastCursor = update.cursor,
                lastRevision = update.revision,
            ),
            applied = true,
        )
    }
}

internal fun decodeAgentCoreTaskUpdate(envelope: JsonObject): AgentCoreTaskUpdate? {
    val event = envelope.objectValue("event") ?: return null
    if (event.stringValue("type") != "task_updated") return null
    val task = event.objectValue("task")?.toAgentCoreTaskHandle() ?: return null
    return AgentCoreTaskUpdate(
        sessionId = envelope.stringValue("session_id") ?: return null,
        turnId = event.stringValue("turn_id")
            ?: envelope.stringValue("turn_id")
            ?: task.ownerTurnId,
        cursor = envelope.objectValue("cursor")?.longValue("sequence") ?: return null,
        revision = envelope.longValue("revision") ?: return null,
        taskProjectionRevision = event.longValue("task_projection_revision") ?: return null,
        task = task,
        guidance = event.objectValue("guidance")?.toAgentCoreTaskGuidanceQueue(),
    )
}

internal fun decodeAgentCoreTaskProjection(
    value: JsonElement?,
    expectedSessionId: String,
): AgentCoreTaskProjection? {
    val source = value as? JsonObject ?: return null
    val revision = source.longValue("revision") ?: 0L
    val taskObjects = source.objectValue("tasks") ?: JsonObject(emptyMap())
    val tasks = taskObjects.mapValues { (_, element) ->
        (element as? JsonObject)?.toAgentCoreTaskHandle() ?: return null
    }
    if (tasks.values.any { it.ownerSessionId != expectedSessionId }) return null
    if (tasks.any { (id, task) -> id != task.id }) return null
    val guidanceObjects = source.objectValue("guidance") ?: JsonObject(emptyMap())
    val guidance = guidanceObjects.mapValues { (taskId, element) ->
        val task = tasks[taskId] ?: return null
        val queue = (element as? JsonObject)?.toAgentCoreTaskGuidanceQueue() ?: return null
        if (queue.pending.any {
                it.parentSessionId != expectedSessionId || it.parentTurnId != task.ownerTurnId
            }
        ) return null
        queue
    }
    val declaredOrder = (source["order"] as? JsonArray)
        ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
        .orEmpty()
    if (declaredOrder.distinct().size != declaredOrder.size) return null
    if (declaredOrder.any { it !in tasks }) return null
    val completeOrder = declaredOrder + tasks.keys.filterNot(declaredOrder::contains)
    return AgentCoreTaskProjection(
        sessionId = expectedSessionId,
        revision = revision,
        order = completeOrder,
        tasks = tasks,
        guidance = guidance,
    )
}

internal fun agentCoreTaskCancellationSubmission(taskId: String): JsonObject = buildJsonObject {
    put("type", "request_task_cancellation")
    put("task_id", taskId)
}

internal fun agentCoreTaskGuidanceSubmission(
    task: AgentCoreTaskHandle,
    content: String,
    messageId: String,
    enqueuedAtEpochMs: Long,
): JsonObject = buildJsonObject {
    put("type", "queue_task_guidance")
    put("task_id", task.id)
    put("message_id", messageId)
    put("parent_session_id", task.ownerSessionId)
    put("parent_turn_id", task.ownerTurnId)
    put("content", content)
    put("enqueued_at_epoch_ms", enqueuedAtEpochMs)
}

private fun JsonObject.toAgentCoreTaskHandle(): AgentCoreTaskHandle? {
    val id = stringValue("id") ?: return null
    val ownerSessionId = stringValue("owner_session_id") ?: return null
    val ownerTurnId = stringValue("owner_turn_id") ?: return null
    val kind = AgentCoreTaskKind.fromWireValue(stringValue("kind")) ?: return null
    val state = AgentCoreTaskState.fromWireValue(stringValue("state")) ?: return null
    return AgentCoreTaskHandle(
        id = id,
        ownerSessionId = ownerSessionId,
        ownerTurnId = ownerTurnId,
        kind = kind,
        state = state,
        progress = (this["progress"] as? JsonPrimitive)?.doubleOrNull,
        currentStep = stringValue("current_step"),
        resultRef = stringValue("result_ref"),
        cancelRequested = (this["cancel_requested"] as? JsonPrimitive)?.booleanOrNull ?: false,
        lineage = objectValue("lineage")?.toAgentCoreTaskLineage()
            ?: AgentCoreTaskLineage(),
        durability = objectValue("durability")?.toAgentCoreTaskDurability()
            ?: AgentCoreTaskDurability(),
    ).takeIf { it.hasValidResultShape }
}

private fun JsonObject.toAgentCoreTaskGuidanceQueue(): AgentCoreTaskGuidanceQueue? {
    val lastSequence = longValue("last_sequence") ?: 0L
    val acknowledgedSequence = longValue("acknowledged_sequence") ?: 0L
    if (lastSequence < 0 || acknowledgedSequence !in 0..lastSequence) return null
    val pending = (this["pending"] as? JsonArray).orEmpty().map { element ->
        val message = element as? JsonObject ?: return null
        AgentCoreTaskGuidanceMessage(
            sequence = message.longValue("sequence") ?: return null,
            messageId = message.stringValue("message_id")?.takeIf { it.isNotBlank() } ?: return null,
            parentSessionId = message.stringValue("parent_session_id")?.takeIf { it.isNotBlank() } ?: return null,
            parentTurnId = message.stringValue("parent_turn_id")?.takeIf { it.isNotBlank() } ?: return null,
            content = message.stringValue("content")?.takeIf { it.isNotBlank() } ?: return null,
            enqueuedAtEpochMs = message.longValue("enqueued_at_epoch_ms") ?: return null,
        )
    }
    if (pending.map { it.sequence }.zipWithNext().any { (left, right) -> right <= left }
        || pending.any { it.sequence <= acknowledgedSequence || it.sequence > lastSequence }
        || pending.map { it.messageId }.distinct().size != pending.size
    ) return null
    val acknowledgedMessageIds = (this["acknowledged_message_ids"] as? JsonArray)
        ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
        .orEmpty()
    if (acknowledgedMessageIds.any { it.isBlank() }
        || acknowledgedMessageIds.distinct().size != acknowledgedMessageIds.size
    ) return null
    return AgentCoreTaskGuidanceQueue(
        lastSequence = lastSequence,
        acknowledgedSequence = acknowledgedSequence,
        pending = pending,
        acknowledgedMessageIds = acknowledgedMessageIds,
    )
}

private fun JsonObject.toAgentCoreTaskLineage(): AgentCoreTaskLineage =
    AgentCoreTaskLineage(
        parentTaskId = stringValue("parent_task_id"),
        childSessionId = stringValue("child_session_id"),
        parentCallId = stringValue("parent_call_id"),
        permissionScopeId = stringValue("permission_scope_id"),
        cancellationScopeId = stringValue("cancellation_scope_id"),
    )

private fun JsonObject.toAgentCoreTaskDurability(): AgentCoreTaskDurability =
    AgentCoreTaskDurability(
        runResult = objectValue("run_result"),
        resultDigest = stringValue("result_digest"),
        terminalReceiptId = stringValue("terminal_receipt_id"),
        workerId = stringValue("worker_id"),
        ownershipToken = stringValue("ownership_token"),
        leaseUntilEpochMs = longValue("lease_until_epoch_ms"),
        lastHeartbeatEpochMs = longValue("last_heartbeat_epoch_ms"),
        retryCount = longValue("retry_count")?.toInt() ?: 0,
        retryAtEpochMs = longValue("retry_at_epoch_ms"),
        lastError = stringValue("last_error"),
        inputWatermark = longValue("input_watermark"),
        completionWatermark = longValue("completion_watermark"),
    )

private fun JsonObject.objectValue(key: String): JsonObject? = this[key] as? JsonObject
private fun JsonObject.stringValue(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.longValue(key: String): Long? =
    (this[key] as? JsonPrimitive)?.longOrNull
