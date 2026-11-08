package com.nola.data.local

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*
import java.util.UUID

/** Private source fences: never render, log, or send to the model. */
@Serializable
data class UnknownEffectRecoveryTarget(
    @SerialName("effect_id") val effectId: String,
    @SerialName("request_id") val requestId: String,
    @SerialName("attempt_id") val attemptId: String,
    @SerialName("worker_generation") val workerGeneration: String,
    @SerialName("lease_owner") val leaseOwner: String,
    @SerialName("lease_token") val leaseToken: String,
    @SerialName("expected_attempt_revision") val expectedAttemptRevision: Long,
) {
    fun validate() {
        require(listOf(effectId, requestId, attemptId, workerGeneration, leaseOwner, leaseToken).all(::validRecoveryIdentity))
        require(expectedAttemptRevision in 1 until Long.MAX_VALUE)
    }
    override fun toString(): String = "UnknownEffectRecoveryTarget(private)"
}

@Serializable
data class UnknownEffectRecoveryInspection(
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("run_id") val runId: String,
    @SerialName("assistant_message_id") val assistantMessageId: String,
    @SerialName("invocation_config_digest") val invocationConfigDigest: String,
    val targets: List<UnknownEffectRecoveryTarget>,
    @SerialName("independent_pending_effects") val independentPendingEffects: Long,
    @SerialName("retry_allowed") val retryAllowed: Boolean,
) {
    fun validate(conversation: String, run: String, assistant: String) {
        require(conversationId == conversation && runId == run && assistantMessageId == assistant)
        require(listOf(conversationId, runId, assistantMessageId, invocationConfigDigest).all(::validRecoveryIdentity))
        require(targets.size <= 256 && independentPendingEffects >= 0)
        targets.forEach { it.validate() }
        require(targets.map { it.effectId }.distinct().size == targets.size)
    }

    fun decision(targetIndex: Int?, reason: String): UnknownEffectRecoveryDecision {
        require(reason.isNotBlank() && reason.toByteArray(Charsets.UTF_8).size <= 4096)
        val selected = if (targetIndex == null) {
            require(independentPendingEffects == 0L && targets.isNotEmpty())
            targets
        } else { require(retryAllowed); listOf(targets[targetIndex]) }
        return UnknownEffectRecoveryDecision(conversationId, runId, invocationConfigDigest,
            UUID.randomUUID().toString(), selected,
            UnknownEffectRecoveryChoice(if (targetIndex == null) UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked
                else UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry, reason), System.currentTimeMillis())
            .also { it.validate() }
    }
    override fun toString(): String = "UnknownEffectRecoveryInspection(private)"
}

@Serializable
data class UnknownEffectRecoveryChoice(val kind: Kind, val reason: String) {
    @Serializable enum class Kind {
        @SerialName("authorize_risk_retry") AuthorizeRiskRetry,
        @SerialName("keep_unknown_blocked") KeepUnknownBlocked,
    }
}

@Serializable
data class UnknownEffectRecoveryDecision(
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("run_id") val runId: String,
    @SerialName("invocation_config_digest") val invocationConfigDigest: String,
    @SerialName("resolution_id") val resolutionId: String,
    val targets: List<UnknownEffectRecoveryTarget>,
    val choice: UnknownEffectRecoveryChoice,
    @SerialName("resolved_at_ms") val resolvedAtMs: Long,
) {
    fun validate() {
        require(listOf(conversationId, runId, invocationConfigDigest, resolutionId).all(::validRecoveryIdentity))
        require(resolvedAtMs >= 0 && targets.size in 1..256)
        require(choice.reason.isNotBlank() && choice.reason.toByteArray(Charsets.UTF_8).size <= 4096)
        require(choice.kind != UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry || targets.size == 1)
        require(targets.map { it.effectId }.distinct().size == targets.size)
        targets.forEach { it.validate() }
    }
    override fun toString(): String = "UnknownEffectRecoveryDecision(private)"
}

@Serializable
data class UnknownEffectRecoveryReply(
    val outcome: Outcome,
    @SerialName("resolution_id") val resolutionId: String? = null,
    @SerialName("run_id") val runId: String? = null,
    val code: String? = null,
) {
    @Serializable enum class Outcome {
        @SerialName("retry_authorized") RetryAuthorized,
        @SerialName("unknown_preserved_blocked") UnknownPreservedBlocked,
        @SerialName("rejected") Rejected,
        @SerialName("conflict") Conflict,
    }
    fun validate(decision: UnknownEffectRecoveryDecision) {
        when (outcome) {
            Outcome.RetryAuthorized, Outcome.UnknownPreservedBlocked -> {
                require(resolutionId == decision.resolutionId && runId == decision.runId && code == null)
                require((outcome == Outcome.RetryAuthorized) ==
                    (decision.choice.kind == UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry))
            }
            Outcome.Rejected, Outcome.Conflict -> require(!code.isNullOrBlank() && resolutionId == null && runId == null)
        }
    }
}

data class PendingUnknownEffectRecovery(val requestId: String, val decision: UnknownEffectRecoveryDecision) {
    override fun toString(): String = "PendingUnknownEffectRecovery(private)"
}

private fun validRecoveryIdentity(value: String): Boolean = value.isNotEmpty() && value.trim() == value &&
    value.toByteArray(Charsets.UTF_8).size <= 512 && value.none { it.isISOControl() }
