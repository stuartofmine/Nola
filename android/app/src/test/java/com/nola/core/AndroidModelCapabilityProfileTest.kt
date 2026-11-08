package com.nola.core

import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.*
import org.junit.Test

class AndroidModelCapabilityProfileTest {
    @Test
    fun unknownLocalModelDefaultsToolsOnWithoutInferringOtherCapabilitiesFromItsName() {
        val profile = androidModelCapabilityProfile(
            ModelInfo("qwen-vision-thinking", "model", source = ModelInfo.ModelSource.Local),
            ModelParameters(), false,
        )
        assertTrue(profile.id.isNotBlank())
        assertEquals(1L, profile.revision)
        val wire = profile.wire()
        assertTrue(wire.getValue("native_tool_calls").jsonPrimitive.boolean)
        for (key in listOf("vision_input", "reasoning", "stream_text",
            "stream_tool_calls", "stream_reasoning", "prompt_cache", "request_continuation")) {
            assertFalse(key, wire.getValue(key).jsonPrimitive.boolean)
        }
    }

    @Test
    fun declaredCapabilitiesAreIntersectedWithRealStreamingSupport() {
        val model = ModelInfo("model", "model", profileId = "selected", profileRevision = 9,
            executionCapabilities = ModelExecutionCapabilities(toolCalls = true, imageInput = true, reasoning = true))
        val params = ModelParameters(maxContextTokens = 32768, maxCompletionTokens = 8192)
        for (streaming in listOf(false, true)) {
            val profile = androidModelCapabilityProfile(model, params, streaming)
            val wire = profile.wire()
            assertEquals(9L, wire.getValue("revision").jsonPrimitive.long)
            assertEquals(32768L, wire.getValue("context_window_tokens").jsonPrimitive.long)
            assertEquals(8192L, wire.getValue("max_output_tokens").jsonPrimitive.long)
            assertTrue(wire.getValue("native_tool_calls").jsonPrimitive.boolean)
        for (key in listOf("vision_input", "reasoning")) {
                assertTrue(key, wire.getValue(key).jsonPrimitive.boolean)
            }
            for (key in listOf("stream_text", "stream_tool_calls", "stream_reasoning")) {
                assertEquals(key, streaming, wire.getValue(key).jsonPrimitive.boolean)
            }
            for (key in listOf("parallel_tool_calls", "prompt_cache", "reasoning_continuity", "request_continuation")) {
                assertFalse(key, wire.getValue(key).jsonPrimitive.boolean)
            }
        }
    }

    @Test
    fun transportCanReceiveReasoningWithoutClaimingGenerationControls() {
        val wire = androidModelCapabilityProfile(
            ModelInfo("ordinary", "model", executionCapabilities = ModelExecutionCapabilities(reasoning = false)),
            ModelParameters(), true,
        ).wire()
        assertFalse(wire.getValue("reasoning").jsonPrimitive.boolean)
        assertTrue(wire.getValue("stream_reasoning").jsonPrimitive.boolean)
        assertFalse(wire.getValue("reasoning_continuity").jsonPrimitive.boolean)
    }

    @Test fun currentCanonicalProfileRoundTripsWithExplicitUnsupportedPdfTransport() {
        val original = androidModelCapabilityProfile(ModelInfo("ordinary", "model"), ModelParameters(), true)
        val canonical = original.wire()
        assertEquals(JsonPrimitive(false), canonical.getValue("native_pdf_input"))
        assertEquals(original, frozenAndroidModelCapability(canonical))
        assertEquals(canonical, frozenAndroidModelCapability(canonical).wire())
    }

    @Test fun nativePdfOrUnknownCapabilityIsNotSilentlyRemovedFromFrozenProfile() {
        val canonical = androidModelCapabilityProfile(ModelInfo("ordinary", "model"), ModelParameters(), true).wire()
        for (unsupported in listOf(
            JsonObject(canonical + ("native_pdf_input" to JsonPrimitive(true))),
            JsonObject(canonical + ("unknown_capability" to JsonPrimitive(true))),
        )) assertTrue(runCatching { frozenAndroidModelCapability(unsupported) }.exceptionOrNull() is IllegalArgumentException)
    }

    @Test(expected = IllegalArgumentException::class)
    fun invalidExplicitRevisionIsNotRepaired() {
        androidModelCapabilityProfile(ModelInfo("model", "model", profileRevision = 0), ModelParameters(), true)
    }
}
