package com.nola.ui.screen.chat

import com.nola.data.remote.sse.SseEvent
import com.nola.llm.provider.WebAgentProvider
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Owns one remote task-projection subscription and its monotonic stream cursor.
 *
 * Input: conversation identity, observation job, and server cursors.
 * Output: whether a subscription is current and the cursor for the next request.
 */
internal class RemoteTaskObservation {
    private var job: Job? = null
    private var conversationId: String? = null
    private var cursor: Long = 0

    fun isActiveFor(conversationId: String): Boolean =
        this.conversationId == conversationId && job?.isActive == true

    fun observe(
        scope: CoroutineScope,
        provider: WebAgentProvider,
        configuration: WebAgentProvider.Config,
        conversationId: String,
        isOwnerActive: () -> Boolean,
        shouldReconnect: () -> Boolean,
        onProjection: (SseEvent.TaskProjection) -> Unit,
    ) {
        if (isActiveFor(conversationId)) return
        this.job?.cancel()
        if (this.conversationId != conversationId) cursor = 0
        this.conversationId = conversationId
        val job = scope.launch(start = CoroutineStart.LAZY) {
            var retryDelayMillis = INITIAL_RETRY_DELAY_MILLIS
            while (isActive && isOwnerActive()) {
                try {
                    provider.taskProjectionEvents(conversationId, cursor, configuration).collect { event ->
                        when (event) {
                            is SseEvent.TaskProjection -> onProjection(event)
                            is SseEvent.TaskStreamCheckpoint -> if (event.conversationId == conversationId) {
                                advance(event.cursor)
                            }
                            else -> Unit
                        }
                        retryDelayMillis = INITIAL_RETRY_DELAY_MILLIS
                    }
                    if (!shouldReconnect()) break
                } catch (cancelled: kotlinx.coroutines.CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    delay(retryDelayMillis)
                    retryDelayMillis = (retryDelayMillis * 2).coerceAtMost(MAX_RETRY_DELAY_MILLIS)
                }
            }
            finish(conversationId)
        }
        this.job = job
        job.start()
    }

    fun cursor(): Long = cursor

    fun advance(candidate: Long) {
        cursor = maxOf(cursor, candidate)
    }

    fun finish(conversationId: String) {
        if (this.conversationId == conversationId) {
            job = null
            this.conversationId = null
        }
    }

    fun reset() {
        job?.cancel()
        job = null
        conversationId = null
        cursor = 0
    }

    private companion object {
        const val INITIAL_RETRY_DELAY_MILLIS = 500L
        const val MAX_RETRY_DELAY_MILLIS = 4_000L
    }
}
