package com.nola.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withContext
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.ExperimentalCoroutinesApi
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class OwnedHostEffectCancellationTest {
    @Test fun stopInterruptsSuspendedEffectBeforeWaitingForPhysicalTeardown() = runTest {
        val requests = Channel<String>(1)
        val entered = CompletableDeferred<Unit>()
        val cleanup = CompletableDeferred<Unit>()
        var interrupted = false
        var effectFinished = false
        val result = async {
            awaitOwnedHostEffect(requests, interrupt = { reason ->
                assertEquals("user_requested", reason)
                interrupted = true
                "canonical interrupt"
            }, operation = {
                entered.complete(Unit)
                try { awaitCancellation() }
                finally {
                    withContext(NonCancellable) { cleanup.await() }
                    effectFinished = true
                }
            })
        }
        entered.await(); requests.send("user_requested"); runCurrent()
        assertTrue(interrupted); assertFalse(effectFinished); assertFalse(result.isCompleted)
        cleanup.complete(Unit); runCurrent()
        assertEquals("canonical interrupt", result.await()); assertTrue(effectFinished)
    }

    @Test fun completedEffectDoesNotInventCancellationOrFailure() = runTest {
        val requests = Channel<String>(1)
        val result = awaitOwnedHostEffect(requests,
            interrupt = { fail("No cancellation was requested"); "not reached" },
            operation = { "exact effect receipt" })
        assertEquals("exact effect receipt", result)
    }

    @Test fun collectorCancellationDoesNotCancelDurableRunAndStillWaitsForPhysicalCleanup() = runTest {
        val requests = Channel<String>(1)
        val cleanup = CompletableDeferred<Unit>()
        var interruptReason: String? = null
        val worker = launch {
            awaitOwnedHostEffect(requests,
                interrupt = { interruptReason = it; Unit },
                operation = { try { awaitCancellation() } finally { withContext(NonCancellable) { cleanup.await() } } })
        }
        runCurrent(); worker.cancel(); runCurrent()
        assertNull(interruptReason)
        assertFalse(worker.isCompleted)
        cleanup.complete(Unit); runCurrent(); assertTrue(worker.isCompleted); assertNull(interruptReason)
    }
}
