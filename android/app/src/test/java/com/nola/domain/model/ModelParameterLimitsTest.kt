package com.nola.domain.model

import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.assertFalse
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class ModelParameterLimitsTest {
    @Test fun canonicalRemoteParametersPreserveUnsetAndExplicitSampling() {
        val json = kotlinx.serialization.json.Json { ignoreUnknownKeys = false }
        val value = json.decodeFromString(ModelParameters.serializer(), """{"maxContextLength":128000,"maxOutputTokens":16384,"topK":null,"repetitionPenalty":null,"showThinking":true}""")
        assertEquals(null, value.topK)
        assertEquals(null, value.repetitionPenalty)
        assertEquals(128000, value.maxContextTokens)
        assertEquals(value, json.decodeFromString(ModelParameters.serializer(), json.encodeToString(ModelParameters.serializer(), value)))
        val explicit = value.copy(topK = 40, repetitionPenalty = 1.0)
        assertEquals(explicit, json.decodeFromString(ModelParameters.serializer(), json.encodeToString(ModelParameters.serializer(), explicit)))
    }

    @Test fun validationPreservesExplicitLimitsAndRejectsInvalidValues() {
        for (value in listOf(64, 16_384, 64_000, 1_000_000)) {
            assertEquals(value, ModelParameterLimits.validatedOutputTokens(value))
        }
        for (value in listOf(-1, 0, 63, 1_000_001)) {
            assertFailsWith<IllegalArgumentException> { ModelParameterLimits.validatedOutputTokens(value) }
        }
    }
    @Test fun tokenRangesMatchDesktopAndHostWithoutChangingDefaults() {
        assertTrue(64_000 in ModelParameterLimits.outputTokens)
        assertTrue(1_000_000 in ModelParameterLimits.outputTokens)
        assertFalse(0 in ModelParameterLimits.outputTokens)
        assertTrue(4_000_000 in ModelParameterLimits.remoteContextTokens)
        assertFalse(4_000_001 in ModelParameterLimits.remoteContextTokens)
    }
}
