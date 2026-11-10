package com.nola.work

import android.content.Context
import androidx.hilt.work.HiltWorker
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import com.nola.core.AndroidAgentTurnService
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import kotlinx.coroutines.CancellationException

@HiltWorker
class MemoryWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted workerParameters: WorkerParameters,
    private val agentTurnService: AndroidAgentTurnService,
) : CoroutineWorker(context, workerParameters) {
    override suspend fun doWork(): Result {
        val sessionId = inputData.getString(KEY_SESSION_ID)?.trim().orEmpty()
        if (sessionId.isEmpty()) return Result.failure()
        return try {
            val report = agentTurnService.runPendingMemoryWorkersReport(sessionId)
            when (dispositionForReport(report.retryScheduled, runAttemptCount)) {
                MemoryWorkerDisposition.Success -> Result.success()
                MemoryWorkerDisposition.Retry -> Result.retry()
                MemoryWorkerDisposition.Failure -> Result.failure()
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            if (runAttemptCount < MAXIMUM_ATTEMPTS - 1) Result.retry() else Result.failure()
        }
    }

    companion object {
        const val KEY_SESSION_ID = "session_id"
        // Core allows three retries after the initial claim. WorkManager's
        // runAttemptCount is zero-based and must therefore admit four runs.
        internal const val MAXIMUM_ATTEMPTS = 4

        internal fun dispositionForReport(
            retryScheduled: Int,
            runAttemptCount: Int,
        ): MemoryWorkerDisposition =
            if (retryScheduled <= 0) {
                MemoryWorkerDisposition.Success
            } else if (runAttemptCount < MAXIMUM_ATTEMPTS - 1) {
                MemoryWorkerDisposition.Retry
            } else {
                MemoryWorkerDisposition.Failure
            }
    }
}

internal enum class MemoryWorkerDisposition { Success, Retry, Failure }
