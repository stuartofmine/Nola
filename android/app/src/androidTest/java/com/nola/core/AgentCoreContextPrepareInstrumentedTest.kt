package com.nola.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AgentCoreContextPrepareInstrumentedTest {
    @Test
    fun nativePrepareReturnsVersionedMetrics() {
        val output = AgentCoreContextPreparation.prepare(
            AgentCoreContextPrepareInput(
                messages = listOf(
                    buildJsonObject {
                        put("id", "m1")
                        put("role", "user")
                        put("content", "hello")
                    }
                ),
                budget = AgentCoreContextPrepareBudget(contextWindowTokens = 8_192),
            )
        )

        assertNotNull(output)
        assertEquals(2, output?.schemaVersion)
        assertEquals("ready", output?.action)
        assertNotNull(output?.metrics)
        assertNotNull(output?.payloadDigest)
    }
}
