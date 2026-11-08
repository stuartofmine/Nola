// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Strictly projects authoritative Core Skill runtime events on Android.
// - Safety: Text, Trace, and ordinary tool events cannot create or mutate Skill runs.

package com.nola.core

import com.nola.domain.model.HostAgentToolEvent
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.longOrNull

enum class AgentCoreSkillRunStatus(val wireValue: String) {
    Running("running"),
    WaitingForCatalog("waiting_for_catalog"),
    Completed("completed"),
    Failed("failed"),
    Cancelled("cancelled");

    val isTerminal: Boolean get() = this == Completed || this == Failed || this == Cancelled

    companion object {
        fun fromWireValue(value: String?): AgentCoreSkillRunStatus? = entries.firstOrNull {
            it.wireValue == value
        }
    }
}

data class AgentCoreSkillRunUsage(
    val modelTurns: Long,
    val toolCalls: Long,
    val elapsedMillis: Long,
) {
    fun isMonotonicAfter(old: AgentCoreSkillRunUsage): Boolean =
        modelTurns >= old.modelTurns &&
            toolCalls >= old.toolCalls &&
            elapsedMillis >= old.elapsedMillis
}

data class AgentCoreSkillRun(
    val runId: String,
    val ownerSessionId: String,
    val ownerParentTurnId: String,
    val skillId: String,
    val version: String,
    val instructionsRef: String,
    val instructionsDigest: String,
    val displayName: String,
    val mode: String,
    val status: AgentCoreSkillRunStatus,
    val usage: AgentCoreSkillRunUsage,
    val catalogRevision: Long,
    val resultRef: String? = null,
    val failure: String? = null,
    val executionContract: AgentCoreSkillExecutionContract,
    val currentProgressStep: String? = null,
    val completedProgressSteps: Set<String> = emptySet(),
    val artifacts: Map<String, String> = emptyMap(),
    val verifications: Map<String, AgentCoreSkillVerificationResult> = emptyMap(),
    val resumePoint: String? = null,
)

data class AgentCoreSkillExecutionContract(
    val progressSteps: List<String>,
    val artifacts: Set<String>,
    val verifiers: Map<String, String>,
    val resumePoints: Set<String>,
)

data class AgentCoreSkillEvidenceSource(val kind: String, val id: String)
data class AgentCoreSkillVerificationEvidence(
    val kind: String,
    val sources: List<AgentCoreSkillEvidenceSource>,
    val resourceUri: String? = null,
    val version: String? = null,
    val checkKind: String? = null,
    val predicate: String? = null,
)
data class AgentCoreSkillVerificationResult(
    val verifierId: String,
    val passed: Boolean,
    val evidence: AgentCoreSkillVerificationEvidence,
    val replacesEvidence: AgentCoreSkillVerificationEvidence?,
    val summary: String,
)

sealed interface AgentCoreSkillRuntimeUpdate {
    data class CatalogReplaced(val revision: Long) : AgentCoreSkillRuntimeUpdate
    data class Started(val run: AgentCoreSkillRun) : AgentCoreSkillRuntimeUpdate
    data class CapabilityRefreshRequested(val runId: String) : AgentCoreSkillRuntimeUpdate
    data class Resumed(val runId: String, val catalogRevision: Long) : AgentCoreSkillRuntimeUpdate
    data class UsageUpdated(
        val runId: String,
        val usage: AgentCoreSkillRunUsage,
    ) : AgentCoreSkillRuntimeUpdate
    data class BudgetExceeded(val runId: String) : AgentCoreSkillRuntimeUpdate
    data class ProgressUpdated(val runId: String, val stepId: String, val completed: Boolean) : AgentCoreSkillRuntimeUpdate
    data class ArtifactRegistered(val runId: String, val artifactId: String, val artifactRef: String) : AgentCoreSkillRuntimeUpdate
    data class VerificationRecorded(val runId: String, val result: AgentCoreSkillVerificationResult) : AgentCoreSkillRuntimeUpdate
    data class ResumePointUpdated(val runId: String, val resumePoint: String) : AgentCoreSkillRuntimeUpdate
    data class Completed(val runId: String, val resultRef: String?) : AgentCoreSkillRuntimeUpdate
    data class Failed(val runId: String, val error: String) : AgentCoreSkillRuntimeUpdate
    data class Cancelled(val runId: String, val reason: String?) : AgentCoreSkillRuntimeUpdate
}

data class AgentCoreSkillRuntimeEnvelope(
    val sessionId: String,
    val turnId: String?,
    val cursor: Long,
    val journalRevision: Long,
    val skillRuntimeRevision: Long,
    val update: AgentCoreSkillRuntimeUpdate,
)

data class AgentCoreCapabilityActivationEnvelope(
    val sessionId: String,
    val turnId: String?,
    val cursor: Long,
    val journalRevision: Long,
    val source: String,
    val capabilityIds: List<String>,
) {
    fun toHostToolEvent(): HostAgentToolEvent = HostAgentToolEvent(
        id = "core-capability:${turnId ?: sessionId}:$source",
        toolName = source,
        argsPreview = capabilityIds.joinToString(separator = " · "),
        status = "completed",
        resultSummary = capabilityIds.joinToString(separator = "\n"),
    )
}

data class AgentCoreSkillRuntimeProjection(
    val sessionId: String? = null,
    val turnId: String? = null,
    val cursor: Long = 0,
    val revision: Long = 0,
    val journalRevision: Long = 0,
    val catalogRevision: Long = 0,
    val order: List<String> = emptyList(),
    val runs: Map<String, AgentCoreSkillRun> = emptyMap(),
) {
    val orderedRuns: List<AgentCoreSkillRun> get() = order.mapNotNull(runs::get)

    fun mergeIntoToolEvents(events: List<HostAgentToolEvent>): List<HostAgentToolEvent> {
        val merged = events.filterNot { it.id.startsWith(CORE_SKILL_TOOL_PREFIX) }.toMutableList()
        orderedRuns.forEach { run ->
            merged += HostAgentToolEvent(
                id = "$CORE_SKILL_TOOL_PREFIX${run.runId}",
                toolName = run.displayName,
                argsPreview = run.mode,
                status = when (run.status) {
                    AgentCoreSkillRunStatus.Running,
                    AgentCoreSkillRunStatus.WaitingForCatalog -> "running"
                    AgentCoreSkillRunStatus.Completed -> "completed"
                    AgentCoreSkillRunStatus.Failed,
                    AgentCoreSkillRunStatus.Cancelled -> "failed"
                },
                resultSummary = run.resultRef ?: run.failure,
                error = run.status == AgentCoreSkillRunStatus.Failed ||
                    run.status == AgentCoreSkillRunStatus.Cancelled,
            )
        }
        return merged
    }
}

data class AgentCoreSkillRuntimeReduction(
    val projection: AgentCoreSkillRuntimeProjection,
    val applied: Boolean,
)

object AgentCoreSkillRuntimeProjectionReducer {
    fun reduce(
        current: AgentCoreSkillRuntimeProjection,
        envelope: AgentCoreSkillRuntimeEnvelope,
        expectedSessionId: String,
        expectedTurnId: String? = null,
    ): AgentCoreSkillRuntimeReduction {
        if (
            expectedSessionId.isBlank() || envelope.sessionId != expectedSessionId ||
            current.sessionId?.let { it != envelope.sessionId } == true ||
            expectedTurnId?.let { it != envelope.turnId } == true ||
            current.turnId?.let { it != envelope.turnId } == true ||
            envelope.cursor <= current.cursor ||
            envelope.journalRevision < current.journalRevision ||
            envelope.skillRuntimeRevision < current.revision
        ) return AgentCoreSkillRuntimeReduction(current, false)

        var nextCatalogRevision = current.catalogRevision
        var nextOrder = current.order
        var nextRuns = current.runs

        fun activeRun(runId: String): AgentCoreSkillRun? =
            nextRuns[runId]?.takeUnless { it.status.isTerminal }

        when (val update = envelope.update) {
            is AgentCoreSkillRuntimeUpdate.CatalogReplaced -> {
                if (update.revision < nextCatalogRevision) return AgentCoreSkillRuntimeReduction(current, false)
                nextCatalogRevision = update.revision
            }
            is AgentCoreSkillRuntimeUpdate.Started -> {
                val run = update.run
                if (
                    run.runId.isBlank() || run.ownerSessionId != envelope.sessionId ||
                    run.ownerParentTurnId != envelope.turnId || run.runId in nextRuns
                ) return AgentCoreSkillRuntimeReduction(current, false)
                nextOrder = nextOrder + run.runId
                nextRuns = nextRuns + (run.runId to run)
            }
            is AgentCoreSkillRuntimeUpdate.CapabilityRefreshRequested -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(status = AgentCoreSkillRunStatus.WaitingForCatalog))
            }
            is AgentCoreSkillRuntimeUpdate.Resumed -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                if (update.catalogRevision < run.catalogRevision) return AgentCoreSkillRuntimeReduction(current, false)
                nextCatalogRevision = maxOf(nextCatalogRevision, update.catalogRevision)
                nextRuns = nextRuns + (run.runId to run.copy(
                    status = AgentCoreSkillRunStatus.Running,
                    catalogRevision = update.catalogRevision,
                ))
            }
            is AgentCoreSkillRuntimeUpdate.UsageUpdated -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                if (!update.usage.isMonotonicAfter(run.usage)) return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(usage = update.usage))
            }
            is AgentCoreSkillRuntimeUpdate.BudgetExceeded -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(
                    status = AgentCoreSkillRunStatus.Failed,
                    failure = "skill_budget_exceeded",
                ))
            }
            is AgentCoreSkillRuntimeUpdate.ProgressUpdated -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                if (update.stepId !in run.executionContract.progressSteps) return AgentCoreSkillRuntimeReduction(current, false)
                val completed = if (update.completed) run.completedProgressSteps + update.stepId else run.completedProgressSteps - update.stepId
                nextRuns = nextRuns + (run.runId to run.copy(completedProgressSteps = completed,
                    currentProgressStep = if (update.completed) run.executionContract.progressSteps.firstOrNull { it !in completed } else update.stepId))
            }
            is AgentCoreSkillRuntimeUpdate.ArtifactRegistered -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                if (update.artifactId !in run.executionContract.artifacts || update.artifactRef.isBlank() ||
                    run.artifacts[update.artifactId]?.let { it != update.artifactRef } == true) return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(artifacts = run.artifacts + (update.artifactId to update.artifactRef)))
            }
            is AgentCoreSkillRuntimeUpdate.VerificationRecorded -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                val result = update.result
                val old = run.verifications[result.verifierId]
                if (run.executionContract.verifiers[result.verifierId] != result.evidence.kind ||
                    (old == null && result.replacesEvidence != null) ||
                    (old != null && old != result && result.replacesEvidence != old.evidence)) return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(verifications = run.verifications + (result.verifierId to result)))
            }
            is AgentCoreSkillRuntimeUpdate.ResumePointUpdated -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                if (update.resumePoint !in run.executionContract.resumePoints) return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(resumePoint = update.resumePoint))
            }
            is AgentCoreSkillRuntimeUpdate.Completed -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(
                    status = AgentCoreSkillRunStatus.Completed,
                    resultRef = update.resultRef,
                ))
            }
            is AgentCoreSkillRuntimeUpdate.Failed -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(
                    status = AgentCoreSkillRunStatus.Failed,
                    failure = update.error,
                ))
            }
            is AgentCoreSkillRuntimeUpdate.Cancelled -> {
                val run = activeRun(update.runId) ?: return AgentCoreSkillRuntimeReduction(current, false)
                nextRuns = nextRuns + (run.runId to run.copy(
                    status = AgentCoreSkillRunStatus.Cancelled,
                    failure = update.reason,
                ))
            }
        }

        return AgentCoreSkillRuntimeReduction(
            projection = current.copy(
                sessionId = envelope.sessionId,
                turnId = envelope.turnId,
                cursor = envelope.cursor,
                revision = envelope.skillRuntimeRevision,
                journalRevision = envelope.journalRevision,
                catalogRevision = nextCatalogRevision,
                order = nextOrder,
                runs = nextRuns,
            ),
            applied = true,
        )
    }
}

internal fun decodeAgentCoreSkillRuntimeEnvelope(envelope: JsonObject): AgentCoreSkillRuntimeEnvelope? {
    val event = envelope.objectValue("event") ?: return null
    if (event.stringValue("type") != "skill_runtime_updated") return null
    return AgentCoreSkillRuntimeEnvelope(
        sessionId = envelope.stringValue("session_id") ?: return null,
        turnId = event.stringValue("turn_id") ?: envelope.stringValue("turn_id"),
        cursor = envelope.objectValue("cursor")?.longValue("sequence") ?: return null,
        journalRevision = envelope.longValue("revision") ?: return null,
        skillRuntimeRevision = event.longValue("skill_runtime_revision") ?: return null,
        update = event.objectValue("update")?.toSkillRuntimeUpdate() ?: return null,
    )
}

internal fun decodeAgentCoreCapabilityActivationEnvelope(
    envelope: JsonObject,
): AgentCoreCapabilityActivationEnvelope? {
    val event = envelope.objectValue("event") ?: return null
    if (event.stringValue("type") != "capabilities_activated") return null
    val capabilityIds = (event["capability_ids"] as? JsonArray)
        ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
        ?.filter(String::isNotBlank)
        ?.distinct()
        .orEmpty()
    if (capabilityIds.isEmpty()) return null
    return AgentCoreCapabilityActivationEnvelope(
        sessionId = envelope.stringValue("session_id") ?: return null,
        turnId = event.stringValue("turn_id") ?: envelope.stringValue("turn_id"),
        cursor = envelope.objectValue("cursor")?.longValue("sequence") ?: return null,
        journalRevision = envelope.longValue("revision") ?: return null,
        source = event.stringValue("source")?.takeIf(String::isNotBlank) ?: return null,
        capabilityIds = capabilityIds,
    )
}

internal fun decodeAgentCoreSkillRuntimeProjection(
    value: JsonElement?,
    expectedSessionId: String,
): AgentCoreSkillRuntimeProjection? {
    val source = value as? JsonObject ?: return null
    val runsObject = source.objectValue("runs") ?: JsonObject(emptyMap())
    val runs = runsObject.mapValues { (_, value) ->
        (value as? JsonObject)?.toSkillRun() ?: return null
    }
    if (runs.any { (id, run) -> id != run.runId || run.ownerSessionId != expectedSessionId }) return null
    return AgentCoreSkillRuntimeProjection(
        sessionId = expectedSessionId,
        revision = source.longValue("revision") ?: 0,
        catalogRevision = source.objectValue("catalog")?.longValue("revision") ?: 0,
        order = runs.keys.sorted(),
        runs = runs,
    )
}

private fun JsonObject.toSkillRuntimeUpdate(): AgentCoreSkillRuntimeUpdate? {
    return when (stringValue("type")) {
        "catalog_replaced" -> AgentCoreSkillRuntimeUpdate.CatalogReplaced(longValue("revision") ?: return null)
        "started" -> AgentCoreSkillRuntimeUpdate.Started(objectValue("run")?.toSkillRun() ?: return null)
        "capability_refresh_requested" -> AgentCoreSkillRuntimeUpdate.CapabilityRefreshRequested(stringValue("run_id") ?: return null)
        "resumed" -> AgentCoreSkillRuntimeUpdate.Resumed(
            stringValue("run_id") ?: return null,
            longValue("catalog_revision") ?: return null,
        )
        "usage_updated" -> AgentCoreSkillRuntimeUpdate.UsageUpdated(
            stringValue("run_id") ?: return null,
            objectValue("usage")?.toSkillUsage() ?: return null,
        )
        "budget_exceeded" -> AgentCoreSkillRuntimeUpdate.BudgetExceeded(stringValue("run_id") ?: return null)
        "progress_updated" -> AgentCoreSkillRuntimeUpdate.ProgressUpdated(stringValue("run_id") ?: return null,
            stringValue("step_id") ?: return null, booleanValue("completed") ?: return null)
        "artifact_registered" -> AgentCoreSkillRuntimeUpdate.ArtifactRegistered(stringValue("run_id") ?: return null,
            stringValue("artifact_id") ?: return null, stringValue("artifact_ref") ?: return null)
        "verification_recorded" -> AgentCoreSkillRuntimeUpdate.VerificationRecorded(stringValue("run_id") ?: return null,
            objectValue("result")?.toVerificationResult() ?: return null)
        "resume_point_updated" -> AgentCoreSkillRuntimeUpdate.ResumePointUpdated(stringValue("run_id") ?: return null,
            stringValue("resume_point") ?: return null)
        "completed" -> AgentCoreSkillRuntimeUpdate.Completed(stringValue("run_id") ?: return null, stringValue("result_ref"))
        "failed" -> AgentCoreSkillRuntimeUpdate.Failed(stringValue("run_id") ?: return null, stringValue("error") ?: return null)
        "cancelled" -> AgentCoreSkillRuntimeUpdate.Cancelled(stringValue("run_id") ?: return null, stringValue("reason"))
        else -> null
    }
}

private fun JsonObject.toSkillRun(): AgentCoreSkillRun? {
    val owner = objectValue("owner") ?: return null
    val descriptor = objectValue("descriptor") ?: return null
    val digest = descriptor.stringValue("instructions_digest") ?: return null
    if (digest.length != 64 || digest.any { it !in "0123456789abcdef" } ||
        descriptor.stringValue("instructions_ref")?.endsWith("#sha256=$digest") != true) return null
    return AgentCoreSkillRun(
        runId = stringValue("run_id") ?: return null,
        ownerSessionId = owner.stringValue("session_id") ?: return null,
        ownerParentTurnId = owner.stringValue("parent_turn_id") ?: return null,
        skillId = descriptor.stringValue("skill_id") ?: return null,
        version = descriptor.stringValue("version") ?: return null,
        instructionsRef = descriptor.stringValue("instructions_ref") ?: return null,
        instructionsDigest = digest,
        displayName = descriptor.stringValue("display_name") ?: return null,
        mode = stringValue("mode") ?: return null,
        status = AgentCoreSkillRunStatus.fromWireValue(stringValue("status")) ?: return null,
        usage = objectValue("usage")?.toSkillUsage() ?: return null,
        catalogRevision = longValue("catalog_revision") ?: 0,
        resultRef = stringValue("result_ref"),
        failure = stringValue("failure"),
        executionContract = descriptor.objectValue("execution_contract")?.toExecutionContract() ?: return null,
        currentProgressStep = stringValue("current_progress_step"),
        completedProgressSteps = stringArray("completed_progress_steps")?.toSet() ?: return null,
        artifacts = objectValue("artifacts")?.mapValues { (_, value) ->
            (value as? JsonPrimitive)?.contentOrNull ?: return null
        } ?: return null,
        verifications = objectValue("verifications")?.mapValues { (_, value) ->
            (value as? JsonObject)?.toVerificationResult() ?: return null
        } ?: return null,
        resumePoint = stringValue("resume_point"),
    )
}

private fun JsonObject.toExecutionContract(): AgentCoreSkillExecutionContract? {
    if (longValue("schemaVersion") != 1L) return null
    fun entries(key: String): List<JsonObject>? = (this[key] as? JsonArray)?.map { it as? JsonObject ?: return null }
    return AgentCoreSkillExecutionContract(
        progressSteps = entries("progressSteps")?.map { it.stringValue("id") ?: return null } ?: return null,
        artifacts = entries("artifacts")?.map { it.stringValue("id") ?: return null }?.toSet() ?: return null,
        verifiers = entries("verifiers")?.associate { verifier ->
            val method = verifier.stringValue("method") ?: return null
            if (method !in setOf("model_judgment", "host_check")) return null
            (verifier.stringValue("id") ?: return null) to method
        } ?: return null,
        resumePoints = stringArray("resumePoints")?.toSet() ?: return null,
    )
}

private fun JsonObject.toEvidenceSource(): AgentCoreSkillEvidenceSource? {
    val kind = stringValue("kind") ?: return null
    val key = when (kind) { "execution_evidence" -> "evidence_id"; "tool_call" -> "call_id"; else -> return null }
    if (keys != setOf("kind", key)) return null
    val id = stringValue(key) ?: return null
    if (id.isBlank() || id != id.trim() || id.toByteArray(Charsets.UTF_8).size > 256) return null
    return AgentCoreSkillEvidenceSource(kind, id)
}

private fun JsonObject.toVerificationEvidence(): AgentCoreSkillVerificationEvidence? {
    return when (val kind = stringValue("kind")) {
        "model_judgment" -> {
            if (keys != setOf("kind", "sources")) return null
            val sources = (this["sources"] as? JsonArray)?.map { (it as? JsonObject)?.toEvidenceSource() ?: return null } ?: return null
            if (sources.size > 16) return null
            AgentCoreSkillVerificationEvidence(kind, sources)
        }
        "host_check" -> {
            if (keys != setOf("kind", "source", "resource_uri", "version", "check_kind", "predicate")) return null
            val check = stringValue("check_kind") ?: return null
            if (check !in setOf("process_exit", "postcondition")) return null
            AgentCoreSkillVerificationEvidence(kind, listOf(objectValue("source")?.toEvidenceSource() ?: return null),
                stringValue("resource_uri")?.takeIf(String::isNotBlank) ?: return null,
                stringValue("version")?.takeIf(String::isNotBlank) ?: return null, check,
                stringValue("predicate")?.takeIf(String::isNotBlank) ?: return null)
        }
        else -> null
    }
}

private fun JsonObject.toVerificationResult(): AgentCoreSkillVerificationResult? {
    if (keys.any { it !in setOf("verifierId", "passed", "evidence", "replacesEvidence", "summary") }) return null
    val replacement = this["replacesEvidence"]
    return AgentCoreSkillVerificationResult(stringValue("verifierId") ?: return null,
        booleanValue("passed") ?: return null, objectValue("evidence")?.toVerificationEvidence() ?: return null,
        if (replacement == null || replacement == kotlinx.serialization.json.JsonNull) null
        else (replacement as? JsonObject)?.toVerificationEvidence() ?: return null,
        stringValue("summary") ?: return null)
}

private fun JsonObject.stringArray(key: String): List<String>? =
    (this[key] as? JsonArray)?.map { (it as? JsonPrimitive)?.contentOrNull ?: return null }
private fun JsonObject.booleanValue(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull

private fun JsonObject.toSkillUsage(): AgentCoreSkillRunUsage? {
    return AgentCoreSkillRunUsage(
        modelTurns = longValue("model_turns") ?: return null,
        toolCalls = longValue("tool_calls") ?: return null,
        elapsedMillis = longValue("elapsed_millis") ?: return null,
    )
}

private fun JsonObject.objectValue(key: String): JsonObject? = this[key] as? JsonObject
private fun JsonObject.stringValue(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.longValue(key: String): Long? =
    (this[key] as? JsonPrimitive)?.longOrNull

private const val CORE_SKILL_TOOL_PREFIX = "core-skill:"
