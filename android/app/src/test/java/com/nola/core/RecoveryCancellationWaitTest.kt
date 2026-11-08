package com.nola.core

import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class RecoveryCancellationWaitTest {
    @Test fun stopWakesLongColdLeaseWaitWithoutAdvancingLeaseClock() = runTest {
        val stops = Channel<String>(Channel.UNLIMITED)
        val waiting = async { awaitRecoveryCancellation(stops, 240_000) }
        runCurrent()
        assertFalse(waiting.isCompleted)
        stops.send("user_requested")
        runCurrent()
        assertEquals("user_requested", waiting.await())
        assertEquals(0L, testScheduler.currentTime)
        assertTrue(stops.tryReceive().isFailure)
    }

    @Test fun alreadyQueuedStopIsObservedBeforeInspectingSchedule() = runTest {
        val stops = Channel<String>(Channel.UNLIMITED)
        stops.send("user_requested")
        assertEquals("user_requested", awaitRecoveryCancellation(stops, 0))
        assertNull(awaitRecoveryCancellation(stops, 0))
    }

    @Test fun absentStopWaitsUntilExistingLeaseWake() = runTest {
        assertNull(awaitRecoveryCancellation(Channel(Channel.UNLIMITED), 5_000))
        assertEquals(5_000L, testScheduler.currentTime)
    }
}
