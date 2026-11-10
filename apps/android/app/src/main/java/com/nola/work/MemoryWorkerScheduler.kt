package com.nola.work

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import dagger.hilt.android.qualifiers.ApplicationContext
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MemoryWorkerScheduler @Inject constructor(
    @ApplicationContext private val context: Context,
) {
    fun enqueue(sessionId: String) {
        val stableSessionId = sessionId.trim()
        if (stableSessionId.isEmpty()) return
        val request = OneTimeWorkRequestBuilder<MemoryWorker>()
            .setInputData(workDataOf(MemoryWorker.KEY_SESSION_ID to stableSessionId))
            // Core records failed jobs with a 60-second retry horizon. Waking
            // earlier would observe no claimable work and incorrectly finish
            // the unique WorkManager chain as successful.
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, RETRY_BACKOFF_SECONDS, TimeUnit.SECONDS)
            .addTag(TAG)
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork(
            uniqueWorkName(stableSessionId),
            ExistingWorkPolicy.APPEND_OR_REPLACE,
            request,
        )
    }

    companion object {
        private const val TAG = "memory_workers"
        internal const val RETRY_BACKOFF_SECONDS = 60L

        fun uniqueWorkName(sessionId: String): String {
            val digest = MessageDigest.getInstance("SHA-256")
                .digest(sessionId.encodeToByteArray())
                .joinToString(separator = "") { byte -> "%02x".format(byte) }
            return "memory_worker_$digest"
        }
    }
}
