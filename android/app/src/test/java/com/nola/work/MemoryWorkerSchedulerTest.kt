package com.nola.work

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MemoryWorkerSchedulerTest {
    @Test
    fun uniqueWorkNameIsStableAndCollisionResistant() {
        val first = MemoryWorkerScheduler.uniqueWorkName("conversation-a")

        assertEquals(first, MemoryWorkerScheduler.uniqueWorkName("conversation-a"))
        assertTrue(first.startsWith("memory_worker_"))
        assertEquals("memory_worker_".length + 64, first.length)
        // "Aa" and "BB" have the same Java String.hashCode().
        assertNotEquals(
            MemoryWorkerScheduler.uniqueWorkName("Aa"),
            MemoryWorkerScheduler.uniqueWorkName("BB"),
        )
    }

    @Test
    fun coreRetryWaitingStateKeepsWorkManagerChainAliveUntilAttemptLimit() {
        assertEquals(
            MemoryWorkerDisposition.Success,
            MemoryWorker.dispositionForReport(retryScheduled = 0, runAttemptCount = 0),
        )
        assertEquals(
            MemoryWorkerDisposition.Retry,
            MemoryWorker.dispositionForReport(retryScheduled = 1, runAttemptCount = 0),
        )
        assertEquals(
            MemoryWorkerDisposition.Retry,
            MemoryWorker.dispositionForReport(
                retryScheduled = 1,
                runAttemptCount = MemoryWorker.MAXIMUM_ATTEMPTS - 2,
            ),
        )
        assertEquals(
            MemoryWorkerDisposition.Failure,
            MemoryWorker.dispositionForReport(
                retryScheduled = 1,
                runAttemptCount = MemoryWorker.MAXIMUM_ATTEMPTS - 1,
            ),
        )
        assertEquals(60L, MemoryWorkerScheduler.RETRY_BACKOFF_SECONDS)
    }
}
