package com.nola.core

import com.nola.data.local.AndroidApplicationRuntime
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.cancelAndJoin
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/**
 * Executes Core-authored memory work through the canonical Application port.
 * It owns provider I/O only; readiness, leases, retries, plans, and durable
 * state transitions remain Core/Application responsibilities.
 */
class AndroidCanonicalMemoryWorkerService(
    private val application: AndroidApplicationRuntime,
) {
    suspend fun run(
        conversationId: String,
        coreConfig: JsonObject,
        modelClient: AgentModelClient,
        maximumJobCount: Int = 2,
    ): AndroidMemoryWorkerRunReport {
        require(conversationId.isNotBlank()) { "A conversation ID is required" }
        if (maximumJobCount <= 0) return AndroidMemoryWorkerRunReport()
        val jobs = application.execute(buildJsonObject {
            put("type", "query_claimable_memory_jobs")
            put("input", buildJsonObject {
                put("conversation_id", conversationId)
                put("now_epoch_ms", System.currentTimeMillis())
            })
        }).jsonArray
        val workerId = "android-memory-worker-${UUID.randomUUID()}"
        var completed = 0
        var retryScheduled = 0
        jobs.take(maximumJobCount).forEach { value ->
            val jobId = value.jsonObject["id"]?.jsonPrimitive?.contentOrNull
                ?.takeIf(String::isNotBlank)
                ?: error("Core returned a Memory job without an id")
            val ownershipToken = UUID.randomUUID().toString()
            val claimedAt = System.currentTimeMillis()
            commit(conversationId, coreConfig, buildJsonObject {
                put("type", "claim_memory_worker")
                put("operation_id", "android-memory-claim-${UUID.randomUUID()}")
                put("job_id", jobId)
                put("worker_id", workerId)
                put("ownership_token", ownershipToken)
                put("now_epoch_ms", claimedAt)
                put("lease_until_epoch_ms", claimedAt + LEASE_MILLISECONDS)
            })
            try {
                val plan = application.execute(buildJsonObject {
                    put("type", "build_memory_worker_plan")
                    put("input", buildJsonObject {
                        put("conversation_id", conversationId)
                        put("job_id", jobId)
                    })
                }).jsonObject
                val watermark = plan["input_watermark"]?.jsonPrimitive?.contentOrNull
                    ?: error("Core returned a Memory Worker plan without input_watermark")
                val output = executePlan(
                    conversationId = conversationId,
                    coreConfig = coreConfig,
                    jobId = jobId,
                    ownershipToken = ownershipToken,
                    plan = plan,
                    modelClient = modelClient,
                )
                commit(conversationId, coreConfig, buildJsonObject {
                    put("type", "memory_worker_completed")
                    put("operation_id", "android-memory-complete-${UUID.randomUUID()}")
                    put("job_id", jobId)
                    put("ownership_token", ownershipToken)
                    put("now_epoch_ms", System.currentTimeMillis())
                    put("completion_watermark", watermark)
                    put("output", output)
                })
                completed += 1
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                commit(conversationId, coreConfig, buildJsonObject {
                    put("type", "memory_worker_failed")
                    put("operation_id", "android-memory-fail-${UUID.randomUUID()}")
                    put("job_id", jobId)
                    put("ownership_token", ownershipToken)
                    put("now_epoch_ms", System.currentTimeMillis())
                    put("retry_after_ms", RETRY_MILLISECONDS)
                    put("max_retry_count", MAXIMUM_RETRY_COUNT)
                    put("error", error.message ?: "Memory Worker execution failed")
                })
                retryScheduled += 1
            }
        }
        return AndroidMemoryWorkerRunReport(
            completed = completed,
            retryScheduled = retryScheduled,
        )
    }

    private suspend fun executePlan(
        conversationId: String,
        coreConfig: JsonObject,
        jobId: String,
        ownershipToken: String,
        plan: JsonObject,
        modelClient: AgentModelClient,
    ): JsonElement = coroutineScope {
        val heartbeat = launch {
            while (true) {
                delay(HEARTBEAT_MILLISECONDS)
                val now = System.currentTimeMillis()
                commit(conversationId, coreConfig, buildJsonObject {
                    put("type", "heartbeat_memory_worker")
                    put("operation_id", "android-memory-heartbeat-${UUID.randomUUID()}")
                    put("job_id", jobId)
                    put("ownership_token", ownershipToken)
                    put("now_epoch_ms", now)
                    put("lease_until_epoch_ms", now + LEASE_MILLISECONDS)
                })
            }
        }
        try {
            AgentMemoryWorkerModelExecutor.execute(plan, modelClient)
        } finally {
            heartbeat.cancelAndJoin()
        }
    }

    private fun commit(conversationId: String, coreConfig: JsonObject, submission: JsonObject) {
        application.execute(buildJsonObject {
            put("type", "reduce_and_commit_idle_submission")
            put("input", buildJsonObject {
                put("conversation_id", conversationId)
                put("core_config", coreConfig)
                put("submission", submission)
                put("committed_at_ms", System.currentTimeMillis())
            })
        })
    }

    private companion object {
        const val LEASE_MILLISECONDS = 15 * 60 * 1_000L
        const val HEARTBEAT_MILLISECONDS = 5 * 60 * 1_000L
        const val RETRY_MILLISECONDS = 60 * 1_000L
        const val MAXIMUM_RETRY_COUNT = 3
    }
}

data class AndroidMemoryWorkerRunReport(
    val completed: Int = 0,
    val retryScheduled: Int = 0,
)
