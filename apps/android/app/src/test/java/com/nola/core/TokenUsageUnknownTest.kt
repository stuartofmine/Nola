package com.nola.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test

class TokenUsageUnknownTest {
    @Test fun missingAndNullUsageStayUnknownWhileExplicitZeroIsKnown() {
        for (raw in listOf("{}", """{"prompt_tokens":null,"completion_tokens":null,"total_tokens":null}""")) {
            val usage = Json.decodeFromString<AgentCoreTokenUsage>(raw)
            val wire = usage.toCoreUsageJson()
            for (key in listOf("input_tokens", "output_tokens", "total_tokens", "reasoning_tokens", "cached_input_tokens", "cache_write_input_tokens")) assertEquals(JsonNull, wire[key])
            assertFalse(wire.containsKey("cached_input_tokens_reported"))
            assertEquals("1", wire["request_count"]!!.jsonPrimitive.content)
        }
        val zero = AgentCoreTokenUsage(promptTokens = 0, completionTokens = 0, totalTokens = 0, cachedInputTokens = 0).toCoreUsageJson()
        assertEquals("0", zero["total_tokens"]!!.jsonPrimitive.content)
        assertEquals("0", zero["cached_input_tokens"]!!.jsonPrimitive.content)
    }

    @Test fun partialEvidenceDoesNotManufactureTotalOrReasoning() {
        val wire = AgentCoreTokenUsage(promptTokens = 12).toCoreUsageJson()
        assertEquals(JsonNull, wire["total_tokens"])
        assertEquals(JsonNull, wire["output_tokens"])
        assertEquals(JsonNull, wire["reasoning_tokens"])
        assertEquals("12", wire["known_token_lower_bound"]!!.jsonPrimitive.content)
    }
}
