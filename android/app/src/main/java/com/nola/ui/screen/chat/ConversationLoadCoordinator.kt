package com.nola.ui.screen.chat

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Owns request generations and the optional history lookup tied to a conversation load.
 *
 * Input: a new load/invalidation signal and a candidate request generation.
 * Output: monotonically increasing generations and an authoritative commit decision.
 */
internal class ConversationLoadCoordinator {
    private var generation: Long = 0
    private var compactionSourceLookup: Job? = null

    fun beginRequest(): Long {
        cancelCompactionLookup()
        generation += 1
        return generation
    }

    fun invalidate(): Long {
        generation += 1
        return generation
    }

    fun currentGeneration(): Long = generation

    // Observation is admitted by the selected load identity, not by whether
    // that same load painted Streaming and disabled the input control.
    fun isCurrent(requestedGeneration: Long): Boolean = requestedGeneration == generation

    fun shouldCommit(requestedGeneration: Long): Boolean =
        ConversationLoadCommitGate.shouldCommit(
            requestedGeneration = requestedGeneration,
            currentGeneration = generation,
        )

    fun startCompactionLookup(
        scope: CoroutineScope,
        block: suspend CoroutineScope.() -> Unit,
    ) {
        compactionSourceLookup?.cancel()
        val job = scope.launch(start = CoroutineStart.LAZY, block = block)
        compactionSourceLookup = job
        job.start()
    }

    fun cancelCompactionLookup() {
        compactionSourceLookup?.cancel()
        compactionSourceLookup = null
    }
}
