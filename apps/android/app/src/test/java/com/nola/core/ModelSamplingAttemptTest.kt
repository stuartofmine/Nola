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
}
