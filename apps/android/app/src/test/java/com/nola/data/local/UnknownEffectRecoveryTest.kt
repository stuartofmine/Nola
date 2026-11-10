package com.nola.data.local

import com.nola.ui.screen.chat.UnknownEffectRecoveryPresentation
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class UnknownEffectRecoveryTest {
    private fun inspection(independent: Long = 0) = UnknownEffectRecoveryInspection("conversation", "run", "assistant", "digest",
        listOf(UnknownEffectRecoveryTarget("effect", "request", "attempt", "worker", "owner", "private-token", 2)), independent, true)

    @Test fun decisionsBindTheActualAttemptAndCannotCloseIndependentWork() {
        val source = inspection()
        val retry = source.decision(0, "Explicit user decision")
        assertEquals(source.targets, retry.targets)
        assertEquals(UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry, retry.choice.kind)
        assertEquals(UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked, source.decision(null, "Keep uncertainty").choice.kind)
        assertTrue(runCatching { inspection(1).decision(null, "Cannot abandon independent work") }.isFailure)
        assertTrue(runCatching { source.decision(0, " ") }.isFailure)
        assertFalse(retry.toString().contains("private-token"))
    }

    @Test fun staleConversationAndGenerationCannotOwnARecoveryAction() {
        val view = UnknownEffectRecoveryPresentation("conversation", "run", 8, null, inspection(), null)
        assertTrue(view.owns("conversation", 8))
        assertFalse(view.owns("other", 8))
        assertFalse(view.owns("conversation", 9))
        assertTrue(com.nola.domain.model.ProcessingState.AwaitingRecovery.blocksInput)
        assertFalse(com.nola.domain.model.ProcessingState.AwaitingRecovery.isGenerating)
        assertTrue(com.nola.ui.screen.chat.ConversationLoadCommitGate.shouldCommit(
            8, 8))
        assertFalse(com.nola.ui.screen.chat.ConversationLoadCommitGate.shouldCommit(
            8, 9))
    }

    @Test fun repliesMustReferToTheSameFrozenDecisionAndMatchingOutcome() {
        val decision = inspection().decision(0, "Accept risk")
        val reply = UnknownEffectRecoveryReply(UnknownEffectRecoveryReply.Outcome.RetryAuthorized, decision.resolutionId, "run")
        reply.validate(decision)
        assertTrue(runCatching { reply.copy(runId = "other").validate(decision) }.isFailure)
        assertTrue(runCatching { reply.copy(outcome = UnknownEffectRecoveryReply.Outcome.UnknownPreservedBlocked).validate(decision) }.isFailure)
        assertTrue(runCatching { Json.decodeFromString(UnknownEffectRecoveryReply.serializer(),
            """{"outcome":"retry_authorized","resolution_id":"r","run_id":"run","old_status":"completed"}""") }.isFailure)
    }
}
