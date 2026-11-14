package com.nola.core

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertThrows
import org.junit.Test
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals

class AndroidCanonicalAgentRuntimeRecoveryTest {
    @Test fun activePolicyProjectionRequiresExactRunAndCanonicalProfile() {
        val owner = Json.parseToJsonElement("""{"session_id":"core","active_turn_id":"run","permission_projection_revision":8}""").jsonObject
        val response = Json.parseToJsonElement("""{"conversation_id":"conversation","run_id":"run","session_id":"core","active_turn_id":"run","event_cursor":9,"permission_projection_revision":8,"permissions":{"revision":8,"pending":[],"active_policy":{"run_id":"run","configuration_revision":12,"profile":"allow_all"}}}""").jsonObject
        assertEquals(response["permissions"], validateControlPermissionProjection(response, "conversation", "run", owner, "9"))
        for (policy in listOf("""{"run_id":"other","configuration_revision":12,"profile":"allow_all"}""",
            """{"run_id":"run","configuration_revision":0,"profile":"allow_all"}""",
            """{"run_id":"run","configuration_revision":12,"profile":"recommended"}""")) {
            val permissions = response.getValue("permissions").jsonObject
            assertThrows(IllegalArgumentException::class.java) {
                validateControlPermissionProjection(JsonObject(response + ("permissions" to JsonObject(permissions +
                    ("active_policy" to Json.parseToJsonElement(policy))))), "conversation", "run", owner, "9")
            }
        }
    }

    @Test
    fun localTerminalCompletionRequiresExactCommittedSourceSeal() {
        val descriptor = AndroidCanonicalTurnDescriptor("conversation", "Title", "run", "user", "Hello", null,
            assistantMessageId = "original-assistant")
        val identity = Json.parseToJsonElement("""{"source_store_epoch":"epoch","conversation_id":"conversation","run_id":"run","assistant_message_id":"original-assistant"}""").jsonObject
        val result = Json.parseToJsonElement("""{
            "result_id":"decision","conversation_id":"conversation","run_id":"run",
            "originating_submission_id":"run",
            "input":{"watermark":"1","applications_digest":"${"d".repeat(64)}"},
            "decided_at_revision":"2","decision_event_cursor":"8","requirement_revision":"1",
            "evidence_frontier":"7","value":{"kind":"answered","answer_part_refs":["answer"]}
        }""").jsonObject
        val seal = JsonObject(mapOf(
            "protocol_version" to JsonPrimitive(1), "identity" to identity,
            "admission" to JsonObject(mapOf("identity" to identity, "sequence" to JsonPrimitive("1"),
                "record_digest" to JsonPrimitive("b".repeat(64)))),
            "core_session_id" to JsonPrimitive("distinct-core"), "result" to result,
            "result_digest" to JsonPrimitive("e".repeat(64)), "core_revision" to JsonPrimitive("2"),
            "transition_id" to JsonPrimitive("terminal-transition"), "checkpoint_revision" to JsonPrimitive("2"),
            "event_cursor" to JsonPrimitive("9"), "completed_at_ms" to JsonPrimitive(1000),
            "entry_count" to JsonPrimitive("1"), "heads_count" to JsonPrimitive("1"),
            "child_receipt_count" to JsonPrimitive("0"), "manifest_digest" to JsonPrimitive("c".repeat(64)),
            "seal_digest" to JsonPrimitive("a".repeat(64)),
        ))
        var reads = 0
        val missing = loadAndroidCanonicalTerminalSeal(descriptor, "distinct-core") { query ->
            reads++
            assertEquals(JsonPrimitive("local_assistant_run_terminal_seal"), query["type"])
            assertEquals(JsonPrimitive("conversation"), query["conversation_id"])
            assertEquals(JsonPrimitive("run"), query["run_id"])
            kotlinx.serialization.json.JsonNull
        }
        assertEquals("A trace/EOF without a committed source seal cannot complete", null, missing)
        assertEquals(1, reads)
        val completed = loadAndroidCanonicalTerminalSeal(descriptor, "distinct-core") { seal }
        assertEquals(result, completed?.result)
        assertEquals("original-assistant", completed?.assistantMessageId)
        assertEquals(result,loadAndroidHistoryTerminal(identity) {seal}?.result)
        val foreignEpoch=JsonObject(identity+("source_store_epoch" to JsonPrimitive("other-epoch")))
        assertThrows(IllegalArgumentException::class.java) {loadAndroidHistoryTerminal(foreignEpoch) {seal}}

        for (key in listOf("conversation_id", "run_id", "assistant_message_id", "source_store_epoch")) {
            val changedIdentity = JsonObject(identity + (key to JsonPrimitive(if (key == "source_store_epoch") "" else "foreign")))
            assertThrows(IllegalArgumentException::class.java) {
                loadAndroidCanonicalTerminalSeal(descriptor, "distinct-core") {
                    JsonObject(seal + ("identity" to changedIdentity))
                }
            }
        }
        for ((key, value) in listOf("core_session_id" to "foreign", "result_digest" to "invalid", "seal_digest" to "invalid")) {
            assertThrows(IllegalArgumentException::class.java) {
                loadAndroidCanonicalTerminalSeal(descriptor, "distinct-core") { JsonObject(seal + (key to JsonPrimitive(value))) }
            }
        }
        for (variant in listOf(
            """{"kind":"action_completed","requirement_acceptance_refs":["assessment"]}""",
            """{"kind":"incomplete","reason":{"code":"missing_evidence","detail":null},"remaining_requirement_refs":["r1"],"obligation_refs":[]}""",
            """{"kind":"blocked","reason":{"code":"unavailable","detail":null},"blocking_fact_refs":["fact"],"resolution_condition":"restore resource"}""",
            """{"kind":"failed","error_ref":"error","observed_effect_refs":[],"unknown_effect_refs":[]}""",
            """{"kind":"cancelled","cancellation_id":"cancel","observed_effect_refs":[],"unknown_effect_refs":[]}""",
        )) {
            val expected = JsonObject(result + ("value" to Json.parseToJsonElement(variant)))
            assertEquals("Transport must retain every business result and its evidence", expected,
                loadAndroidCanonicalTerminalSeal(descriptor, "distinct-core") {
                    JsonObject(seal + ("result" to expected))
                }?.result)
        }
    }

    @Test
    fun steeringSuccessorDoesNotReplaceInitialRunOwnerDuringColdRecovery() {
        val visibleLatestAssistant = "successor-message-after-steering"
        val descriptor = loadAndroidRecoveryDescriptor("conversation", "Title", "root-run") { command ->
            assertEquals("active_run", command.getValue("type").toString().trim('"'))
            assertEquals(JsonPrimitive("conversation"), command["conversation_id"])
            Json.parseToJsonElement("""{"run_id":"root-run","assistant_message_id":"opaque-original-owner"}""")
        }
        assertEquals("opaque-original-owner", descriptor.assistantMessageId)
        org.junit.Assert.assertNotEquals(visibleLatestAssistant, descriptor.assistantMessageId)
        assertEquals("root-run", descriptor.submissionId)
        assertEquals("", descriptor.userMessageId)
    }

    @Test
    fun recoveryCannotGuessOwnerWhenRunChangedOrAdmissionOwnerIsMissing() {
        for (active in listOf(
            """{"run_id":"successor-run","assistant_message_id":"other-owner"}""",
            """{"run_id":"root-run","assistant_message_id":null}""",
            """{"run_id":"root-run","assistant_message_id":""}""",
        )) {
            assertThrows(Exception::class.java) {
                loadAndroidRecoveryDescriptor("conversation", "Title", "root-run") { Json.parseToJsonElement(active) }
            }
        }
    }

    @Test
    fun coordinatorConsumesOnlyControlWithMatchingSeparatePermissions() {
        val valid = Json.parseToJsonElement("""{"control":{"permission_projection_revision":0},"permission_projection":{"revision":0,"pending":[]}}""").jsonObject
        assertEquals(valid["permission_projection"], canonicalControlProjection(valid)?.get("permissions"))
        assertEquals(null, canonicalControlProjection(JsonObject(emptyMap())))
        for (key in listOf("snapshot", "checkpoint")) {
            assertThrows(IllegalArgumentException::class.java) {
                canonicalControlProjection(JsonObject(valid + (key to kotlinx.serialization.json.JsonNull)))
            }
        }
        for (key in listOf("control", "permission_projection")) {
            assertThrows(Exception::class.java) { canonicalControlProjection(JsonObject(valid - key)) }
        }
        val mismatched = Json.parseToJsonElement("""{"revision":1,"pending":[]}""")
        assertThrows(IllegalArgumentException::class.java) {
            canonicalControlProjection(JsonObject(valid + ("permission_projection" to mismatched)))
        }
    }

    @Test
    fun executionReplyRequiresExplicitEventArraysAndRejectsRetiredFields() {
        val valid = Json.parseToJsonElement("""{"events":[],"traces":[]}""").jsonObject
        validateExecutionReplyEnvelope(valid)
        for (key in listOf("events", "traces")) {
            assertThrows(IllegalArgumentException::class.java) {
                validateExecutionReplyEnvelope(JsonObject(valid - key))
            }
            assertThrows(IllegalArgumentException::class.java) {
                validateExecutionReplyEnvelope(JsonObject(valid + (key to JsonPrimitive("invalid"))))
            }
        }
        for (key in listOf("snapshot", "checkpoint")) {
            assertThrows(IllegalArgumentException::class.java) {
                validateExecutionReplyEnvelope(JsonObject(valid + (key to kotlinx.serialization.json.JsonNull)))
            }
        }
    }

    @Test
    fun controlPermissionReadRejectsForeignEmptyOwnerAndPreservesUnsignedCursor() {
        val owner = Json.parseToJsonElement("""{"session_id":"core","active_turn_id":null,"permission_projection_revision":7}""").jsonObject
        val response = Json.parseToJsonElement("""{"conversation_id":"conversation","run_id":"run","session_id":"core","active_turn_id":null,"event_cursor":18446744073709551615,"permission_projection_revision":7,"permissions":{"revision":7,"pending":[]}}""").jsonObject
        assertEquals(response["permissions"], validateControlPermissionProjection(response,"conversation","run",owner,"18446744073709551614"))
        for (key in listOf("run_id","conversation_id","session_id","active_turn_id","permission_projection_revision")) {
            assertThrows(IllegalArgumentException::class.java) {
                validateControlPermissionProjection(JsonObject(response + (key to JsonPrimitive("foreign"))),"conversation","run",owner,"18446744073709551614")
            }
        }
        assertThrows(IllegalArgumentException::class.java) {
            validateControlPermissionProjection(JsonObject(response + ("event_cursor" to JsonPrimitive(1))),"conversation","run",owner,"18446744073709551614")
        }
    }

    @Test
    fun permissionRecoveryRequiresCompleteFencedIdentity() {
        validate(JsonArray(listOf(request())))

        // A permission delivery has no model attempt, and its lease ID is independent.
        validate(JsonArray(listOf(request(deliveryAttemptId = "delivery-2"))))

        val foreignWorker = request(workerGeneration = "generation-old")
        assertThrows(IllegalArgumentException::class.java) {
            validate(JsonArray(listOf(foreignWorker)))
        }

        val foreignSession = request(sessionId = "conversation-other")
        assertThrows(IllegalArgumentException::class.java) {
            validate(JsonArray(listOf(foreignSession)))
        }
    }

    @Test
    fun recoveryAcceptsFencedNonPermissionWorkClaimedByApplication() {
        // This validates only the transport fence. Application claim and the
        // explicit unknown-effect decision remain mandatory production gates.
        validate(JsonArray(listOf(request(payloadType = "execute_tool"))))
    }

    @Test
    fun recoveryRejectsIncompleteLeaseEvenWithoutModelAttempt() {
        val original = request()
        val delivery = original.getValue("delivery") as JsonObject
        for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token", "lease_expires_at_ms")) {
            assertThrows(Exception::class.java) {
                validate(JsonArray(listOf(JsonObject(original + ("delivery" to JsonObject(delivery - key))))))
            }
        }
        assertThrows(IllegalArgumentException::class.java) {
            validate(JsonArray(listOf(JsonObject(original + ("delivery" to JsonObject(delivery +
                ("lease_expires_at_ms" to JsonPrimitive(0))))))))
        }
    }

    @Test
    fun recoveryAndClaimRejectPositiveButExpiredLeaseAtExplicitClock() {
        val original = request()
        val delivery = original.getValue("delivery") as JsonObject
        for (expiry in listOf(999L, 1_000L)) {
            val expired = JsonObject(original + ("delivery" to JsonObject(delivery +
                ("lease_expires_at_ms" to JsonPrimitive(expiry)))))
            assertThrows(IllegalArgumentException::class.java) { validate(JsonArray(listOf(expired))) }
            assertThrows(IllegalArgumentException::class.java) { validatedHostDelivery(expired, nowMs = 1_000L) }
        }
        validatedHostDelivery(original, nowMs = 1_000L)
    }

    private fun validate(requests: JsonArray) = validateRecoveredHostRequests(
        requests = requests,
        runId = "run-1",
        sessionId = "conversation-1",
        workerGeneration = "generation-2",
        leaseOwner = "owner-2",
        nowMs = 1_000L,
    )

    private fun request(
        payloadType: String = "request_permission",
        deliveryAttemptId: String = "attempt-1",
        sessionId: String = "conversation-1",
        workerGeneration: String = "generation-2",
    ): JsonObject = JsonObject(mapOf(
        "request_id" to JsonPrimitive("request-1"),
        "effect_id" to JsonPrimitive("effect-1"),
        "session_id" to JsonPrimitive(sessionId),
        "host_id" to JsonPrimitive("android-device"),
        "delivery" to JsonObject(mapOf(
            "attempt_id" to JsonPrimitive(deliveryAttemptId),
            "worker_generation" to JsonPrimitive(workerGeneration),
            "lease_owner" to JsonPrimitive("owner-2"),
            "lease_token" to JsonPrimitive("token-2"),
            "lease_expires_at_ms" to JsonPrimitive(10_000),
        )),
        "payload" to JsonObject(mapOf("type" to JsonPrimitive(payloadType))),
    ))
}
