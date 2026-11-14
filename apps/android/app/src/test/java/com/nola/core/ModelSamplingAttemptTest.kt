package com.nola.core

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Test

class ModelSamplingAttemptTest {
    private fun request(started: Boolean) = buildJsonObject {
        put("attempt_started", started)
        put("attempt_id", "core-prepared-attempt")
    }

    @Test fun preparedFirstSamplingRetainsCoreIdentity() {
        assertEquals(ModelSamplingAttempt("core-prepared-attempt", false), modelSamplingAttempt(request(false), 0))
    }

    @Test fun coldStartedRequestAndFurtherSamplesNeverReuseCoreAttempt() {
        val recovered = modelSamplingAttempt(request(true), 0)
        val retried = modelSamplingAttempt(request(false), 1)
        assertTrue(recovered.resampling)
        assertTrue(retried.resampling)
        assertNotEquals("core-prepared-attempt", recovered.id)
        assertNotEquals(recovered.id, retried.id)
    }

    @Test(expected = IllegalArgumentException::class)
    fun missingCoreStateCannotBeAssumedPrepared() {
        modelSamplingAttempt(buildJsonObject { put("attempt_id", "a") }, 0)
    }
    @org.junit.Test fun repeatedProviderCallIdsUseTheActualAdmittedAttemptNamespace() {
        val event = AgentModelEvent.ToolCallDelta("tool:provider-reused", "provider-reused", "delegate_expert", "{}")
        val first = modelToolCallDeltaPayload(event, "actual-core-attempt-a")
        val next = modelToolCallDeltaPayload(event, "actual-core-attempt-b")
        org.junit.Assert.assertNotEquals(first["part_id"], next["part_id"])
        org.junit.Assert.assertEquals(first, modelToolCallDeltaPayload(event, "actual-core-attempt-a"))
        org.junit.Assert.assertEquals(kotlinx.serialization.json.JsonPrimitive("provider-reused"), first["call_id"])
        org.junit.Assert.assertEquals(first["call_id"], next["call_id"])
    }

}
