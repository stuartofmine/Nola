package com.nola.ui.screen.chat

import com.nola.data.remote.api.*
import com.nola.data.remote.sse.SseStatusException
import kotlinx.coroutines.*
import kotlinx.serialization.json.JsonObject

/** One execution-owner observer. Selection changes do not cancel its transport;
 * exact durable identity and uncertain resume authority survive foregrounding. */
internal class RemoteConversationObserver {
    private var job: Job? = null
    private var context: String? = null
    private var runningCurrent: (() -> Boolean)? = null
    private var key: RemoteRunKey? = null
    private var resumeAuthority: Pair<RemoteRunKey, String?>? = null

    fun reset() {
        stop(); context = null; key = null; resumeAuthority = null
    }

    fun stop() { job?.cancel(); job = null; runningCurrent = null }

    fun start(scope: CoroutineScope, contextId: String, hostId: String, clientId: String,
        conversationId: String, initialKey: RemoteRunKey?, current: () -> Boolean,
        query: suspend (JsonObject) -> JsonObject,
        catchUp: suspend (RemoteRunObservation) -> Boolean,
        onObservation: suspend (RemoteRunObservation, Boolean) -> Unit,
        onAbsent: () -> Unit, onUnavailable: (Exception) -> Unit,
        resume: suspend (RemoteRunKey) -> Unit,
        pause: suspend (Long) -> Unit = { delay(it) },
        reconcileSubmission: suspend () -> Boolean = { true },
        admittedKey: () -> RemoteRunKey? = { null }) {
        if (context == contextId && job?.isActive == true && runningCurrent?.invoke() == true) return
        stop()
        if (context != contextId) {
            context = contextId; key = initialKey; resumeAuthority = null
        }
        runningCurrent = current
        job = scope.launch {
            var failures = 0
            while (current()) {
                try {
                    ensureActive()
                    if (!reconcileSubmission()) { pause(1_000); continue }
                    admittedKey()?.let { admitted ->
                        require(key == null || key == admitted) { "Submission admission cannot replace an observed Run" }
                        key = admitted
                    }
                    val observation = decodeRemoteRunObservation(
                        query(remoteObservationRequest(hostId, clientId, conversationId, key)), hostId, clientId, conversationId, key)
                    ensureActive(); if (!current()) break
                    if (observation == null) { onAbsent(); break }
                    key = observation.key
                    val terminalCommitted = catchUp(observation)
                    ensureActive(); if (!current()) break
                    onObservation(observation, terminalCommitted)
                    if (observation.recovery == RemoteRunRecovery.Terminal && terminalCommitted) {
                        key = null; resumeAuthority = null
                        if (observation.canSubmit) break
                        // This Run is durably closed, but the conversation may
                        // already have another owner. Discover it without
                        // adopting a latest/cached identity for the old Run.
                    }
                    if (observation.recovery == RemoteRunRecovery.Observe) resumeAuthority = null
                    if (observation.recovery == RemoteRunRecovery.ResumeRequired) {
                        val authority = observation.key to observation.workerGeneration
                        if (resumeAuthority != authority) {
                            // Consume before send: EOF, 409 or a lost acknowledgement
                            // can only return to exact observation, never new submit.
                            resumeAuthority = authority
                            try { resume(observation.key) }
                            catch (cancelled: CancellationException) { throw cancelled }
                            catch (conflict: SseStatusException) {
                                if (conflict.statusCode != 409) throw conflict
                            }
                        }
                    }
                    failures = 0
                    pause(1_000)
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (!current()) break
                    onUnavailable(failure)
                    // Bounded cadence, no busy loop or retry of execution input.
                    failures = (failures + 1).coerceAtMost(6)
                    pause((1_000L shl (failures - 1)).coerceAtMost(30_000))
                }
            }
        }
    }
}
