package com.nola.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AndroidCancellationEvidenceTest {
    private fun proof(): JsonObject = buildJsonObject {
        put("conversation_id", "conversation")
        put("run_id", "run")
        put("core_session_id", "session")
        put("host_id", "android")
        put("cancellation_effect_id", "cancel")
        put("cancellation_request_id", "cancel-request")
        put("target_effect_id", "target")
        put("target_request_id", "target-request")
        put("target_payload_identity", "a".repeat(64))
        put("checkpoint_revision", 7)
        put("revoked_attempts", JsonArray(listOf(buildJsonObject {
            put("attempt_id", "old-attempt")
            put("revision", 3)
        })))
    }

    private fun cancel(proof: JsonElement = JsonNull): JsonObject = buildJsonObject {
        put("host_id", "android")
        put("session_id", "session")
        put("turn_id", "run")
        put("effect_id", "cancel")
        put("request_id", "cancel-request")
        put("payload", buildJsonObject {
            put("type", "cancel")
            put("target_effect_id", "target")
            put("target_not_started", proof)
        })
    }

    private fun target(): JsonObject = buildJsonObject {
        put("host_id", "android")
        put("session_id", "session")
        put("turn_id", "run")
        put("effect_id", "target")
        put("request_id", "target-request")
    }

    @Test
    fun coldAbsenceDoesNotConfirmCancellationButExactApplicationProofCan() {
        val evidence = AndroidCancellationEvidence()
        assertThrows(HostEffectCancellationUncertain::class.java) {
            evidence.requireCancellationProof(cancel(), "conversation")
        }
        evidence.requireCancellationProof(cancel(proof()), "conversation")
        val unclaimed = JsonObject(proof() + ("revoked_attempts" to JsonArray(emptyList())))
        evidence.requireCancellationProof(cancel(unclaimed), "conversation")
    }

    @Test
    fun proofIsStrictAndBoundToConversationRunHostSessionControlAndTarget() {
        val evidence = AndroidCancellationEvidence()
        for (field in listOf("conversation_id", "run_id", "core_session_id", "host_id",
            "cancellation_effect_id", "cancellation_request_id", "target_effect_id")) {
            val foreign = JsonObject(proof() + (field to JsonPrimitive("foreign")))
            assertThrows(RuntimeException::class.java) {
                evidence.requireCancellationProof(cancel(foreign), "conversation")
            }
        }
        for ((field, value) in listOf(
            "target_request_id" to JsonPrimitive(" "),
            "target_request_id" to JsonPrimitive("bad\nidentity"),
            "target_request_id" to JsonPrimitive("🙂".repeat(257)),
            "target_payload_identity" to JsonPrimitive("A".repeat(64)),
            "checkpoint_revision" to JsonPrimitive("7"),
            "checkpoint_revision" to JsonPrimitive(0),
            "revoked_attempts" to JsonNull,
        )) {
            val malformed = JsonObject(proof() + (field to value))
            assertThrows(RuntimeException::class.java) {
                evidence.requireCancellationProof(cancel(malformed), "conversation")
            }
        }
        for (field in proof().keys) {
            assertThrows(RuntimeException::class.java) {
                evidence.requireCancellationProof(cancel(JsonObject(proof() - field)), "conversation")
            }
        }
        assertThrows(RuntimeException::class.java) {
            evidence.requireCancellationProof(cancel(JsonObject(proof() + ("extra" to JsonNull))), "conversation")
        }
        val repeated = buildJsonObject { put("attempt_id", "a"); put("revision", 1) }
        for (attempts in listOf(
            JsonArray(listOf(repeated, repeated)),
            JsonArray(List(257) { buildJsonObject { put("attempt_id", "a-$it"); put("revision", 1) } }),
            JsonArray(listOf(buildJsonObject { put("attempt_id", "a"); put("revision", 0) })),
            JsonArray(listOf(JsonObject(repeated + ("extra" to JsonNull)))),
        )) {
            assertThrows(RuntimeException::class.java) {
                evidence.requireCancellationProof(cancel(JsonObject(proof() + ("revoked_attempts" to attempts))), "conversation")
            }
        }
    }

    @Test
    fun requiredNullableProofIsNotAnOldProtocolFallbackEvenAfterLocalReturn() {
        val evidence = AndroidCancellationEvidence()
        evidence.returnedLocally(target(), "conversation")
        val missing = JsonObject(cancel() + ("payload" to buildJsonObject {
            put("type", "cancel"); put("target_effect_id", "target")
        }))
        assertThrows(RuntimeException::class.java) {
            evidence.requireCancellationProof(missing, "conversation")
        }
        assertThrows(RuntimeException::class.java) {
            evidence.requireCancellationProof(cancel(JsonPrimitive("proof")), "conversation")
        }
        val original = cancel()
        val extra = JsonObject(original + ("payload" to JsonObject(
            (original.getValue("payload") as JsonObject) + ("extra" to JsonNull),
        )))
        assertThrows(RuntimeException::class.java) {
            evidence.requireCancellationProof(extra, "conversation")
        }
        evidence.requireCancellationProof(cancel(), "conversation")
    }

    @Test
    fun currentLocalReturnIsOwnerBoundAndClearedBeforeEveryRedispatch() {
        val evidence = AndroidCancellationEvidence()
        evidence.dispatching(target())
        evidence.returnedLocally(target(), "conversation")
        evidence.requireCancellationProof(cancel(), "conversation")
        for (field in listOf("host_id", "session_id", "turn_id")) {
            assertThrows(HostEffectCancellationUncertain::class.java) {
                evidence.requireCancellationProof(JsonObject(cancel() + (field to JsonPrimitive("foreign"))), "conversation")
            }
        }
        assertThrows(HostEffectCancellationUncertain::class.java) {
            evidence.requireCancellationProof(cancel(), "other-conversation")
        }
        evidence.dispatching(JsonObject(target() + ("request_id" to JsonPrimitive("new-target-request"))))
        assertThrows(HostEffectCancellationUncertain::class.java) {
            evidence.requireCancellationProof(cancel(), "conversation")
        }
    }

    @Test
    fun unknownToolOutcomeWinsOverAFunctionReturnOrConflictingNotStartedProof() {
        val evidence = AndroidCancellationEvidence()
        evidence.outcomeUnknown(target())
        evidence.returnedLocally(target(), "conversation")
        for (control in listOf(cancel(), cancel(proof()))) {
            assertThrows(HostEffectCancellationUncertain::class.java) {
                evidence.requireCancellationProof(control, "conversation")
            }
        }
    }

    @Test
    fun cancelledModelHasNoLocalReturnFactUntilOwnedCoroutineCleanupHasJoined() = runTest {
        val evidence = AndroidCancellationEvidence()
        val requests = Channel<String>(1)
        val entered = CompletableDeferred<Unit>()
        val cleanup = CompletableDeferred<Unit>()
        evidence.dispatching(target())
        val joined = async {
            val result = awaitOwnedHostEffect(requests, interrupt = { "canonical interrupt" }, operation = {
                entered.complete(Unit)
                try { awaitCancellation() } finally { withContext(NonCancellable) { cleanup.await() } }
            })
            evidence.returnedLocally(target(), "conversation")
            result
        }
        entered.await()
        requests.send("user_requested")
        runCurrent()
        assertFalse(joined.isCompleted)
        assertThrows(HostEffectCancellationUncertain::class.java) {
            evidence.requireCancellationProof(cancel(), "conversation")
        }
        cleanup.complete(Unit)
        runCurrent()
        assertEquals("canonical interrupt", joined.await())
        evidence.requireCancellationProof(cancel(), "conversation")
    }
}
