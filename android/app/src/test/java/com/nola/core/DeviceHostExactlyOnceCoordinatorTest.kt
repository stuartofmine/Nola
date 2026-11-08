package com.nola.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceHostExactlyOnceCoordinatorTest {
    @Test
    fun recoveryContinuesPermissionAndDeviceEffectsAtCoreSequenceExactlyOnce() {
        val coordinator = DeviceHostExactlyOnceCoordinator("session-1", "android-device")
        val permission = request(
            requestId = "request-permission",
            effectId = "effect-permission",
            resumeAfter = 4,
            payloadType = "request_permission",
        )
        val device = request(
            requestId = "request-device",
            effectId = "effect-device",
            resumeAfter = 1,
            payloadType = "execute_tool",
        )
        val restored = coordinator.restore(buildJsonObject {
            put("ok", true)
            put("host_requests", JsonArray(listOf(permission, device, permission)))
        })

        assertEquals(listOf(permission, device), restored)
        assertEquals(2, coordinator.pendingCount)
        assertEquals(permission, coordinator.poll())
        assertEquals(null, coordinator.responseAttemptId(permission))
        val renewedDelivery = buildJsonObject {
            put("attempt_id", "attempt-effect-permission")
            put("worker_generation", "worker-generation")
            put("lease_owner", "android-device-worker")
            put("lease_token", "lease-effect-permission")
            put("lease_expires_at_ms", 500000)
        }
        coordinator.bindResponseDelivery(permission, renewedDelivery)
        assertEquals(renewedDelivery, coordinator.responseDelivery(permission))
        assertEquals(5L, coordinator.nextSequence(permission))
        assertEquals(6L, coordinator.nextSequence(permission))
        coordinator.completeEffect("effect-permission")
        assertFalse(coordinator.accept(permission))
        assertEquals(device, coordinator.poll())
        assertEquals(2L, coordinator.nextSequence(device))
    }

    @Test
    fun rejectsWrongRequestEffectBindingsAndLateTerminalWork() {
        val coordinator = DeviceHostExactlyOnceCoordinator("session-1", "android-device")
        val first = request("request-1", "effect-1", 0, "execute_tool")
        assertTrue(coordinator.accept(first))
        assertFails { coordinator.accept(request("request-1", "effect-2", 0, "execute_tool")) }
        assertFails { coordinator.accept(request("request-2", "effect-1", 0, "execute_tool")) }
        assertFails {
            coordinator.accept(
                request("request-other-session", "effect-other-session", 0, "execute_tool", "other-session"),
            )
        }
        assertFails {
            coordinator.accept(
                request("request-other-host", "effect-other-host", 0, "execute_tool", host = "other-host"),
            )
        }

        coordinator.finishTurn()
        assertTrue(coordinator.isEmpty)
        assertFalse(coordinator.accept(request("late-request", "late-effect", 0, "execute_tool")))
    }

    @Test
    fun cancelledTurnClearsPendingHostWorkAndRejectsLateEffects() {
        val journal = DeviceHostExactlyOnceCoordinator("session-1", "android-device")
        assertTrue(journal.accept(request("request-1", "effect-1", 0, "request_permission")))
        journal.finishTurn()
        assertTrue(journal.isEmpty)
        assertEquals(0, journal.pendingCount)
        assertFalse(journal.accept(request("late", "late-effect", 0, "execute_tool")))
        assertEquals(null, journal.poll())
    }

    @Test
    fun modelAttemptAndDeliveryRemainIndependentAcrossHeartbeat() {
        val journal = DeviceHostExactlyOnceCoordinator("session-1", "android-device")
        val request = JsonObject(request("model-request", "model-effect", 0, "model_stream") +
            ("attempt_id" to kotlinx.serialization.json.JsonPrimitive("model-attempt")))
        assertTrue(journal.accept(request))
        journal.bindResponseAttempt(request, "model-retry")
        val lease = journal.responseDelivery(request)
        val renewed = JsonObject(lease + ("lease_expires_at_ms" to kotlinx.serialization.json.JsonPrimitive(500_000L)))
        journal.bindResponseDelivery(request, renewed)
        assertEquals("model-retry", journal.responseAttemptId(request))
        assertEquals(renewed, journal.responseDelivery(request))
        for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token")) {
            assertFails {
                journal.bindResponseDelivery(request, JsonObject(renewed +
                    (key to kotlinx.serialization.json.JsonPrimitive("foreign"))))
            }
            assertEquals(renewed, journal.responseDelivery(request))
        }
        assertFails { journal.bindResponseDelivery(request, lease) }
    }

    @Test
    fun missingOrInvalidDeliveryFieldsAreRejectedWithoutRequiringModelAttempt() {
        val request = request("permission", "permission", 0, "request_permission")
        val lease = request.getValue("delivery").jsonObject
        for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token", "lease_expires_at_ms")) {
            val journal = DeviceHostExactlyOnceCoordinator("session-1", "android-device")
            assertFails { journal.accept(JsonObject(request + ("delivery" to JsonObject(lease - key)))) }
            assertTrue(journal.isEmpty)
        }
    }

    private fun request(
        requestId: String,
        effectId: String,
        resumeAfter: Long,
        payloadType: String,
        session: String = "session-1",
        host: String = "android-device",
    ): JsonObject = buildJsonObject {
        put("request_id", requestId)
        put("effect_id", effectId)
        put("host_id", host)
        put("session_id", session)
        put("resume_after_sequence", resumeAfter)
        put("delivery", buildJsonObject {
            put("attempt_id", "attempt-$effectId")
            put("worker_generation", "worker-generation")
            put("lease_owner", "android-device-worker")
            put("lease_token", "lease-$effectId")
            put("lease_expires_at_ms", 240_000L)
        })
        put("payload", buildJsonObject { put("type", payloadType) })
    }

    private fun assertFails(block: () -> Unit) {
        var failure: Throwable? = null
        try {
            block()
        } catch (error: Throwable) {
            failure = error
        }
        assertNotNull(failure)
    }

}
