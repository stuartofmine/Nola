package com.nola.core

import com.nola.domain.model.ModelWireProtocol
import com.nola.llm.provider.OpenAiProvider
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidHostInvocationTest {
    @Test fun httpIdentityNeverPersistsCredentialsOrQueryAndTracksRevocation() {
        val config = OpenAiProvider.Config("https://EXAMPLE.invalid:443/v1?key=private-query", "private-api-key", "wire-model",
            wireProtocol = ModelWireProtocol.Responses)
        val route = androidHttpModelRoute(config)
        assertEquals("https://example.invalid/v1", route.getValue("endpoint").jsonPrimitive.content)
        assertEquals("max_output_tokens", route.getValue("output_token_field").jsonPrimitive.content)
        // Core serializes the current typed optional account explicitly; exact recovery must match.
        assertEquals(JsonNull, route.getValue("siwc_account"))
        assertFalse(route.toString().contains("private-query"))
        assertFalse(route.toString().contains("private-api-key"))
        assertNotEquals(route, androidHttpModelRoute(config.copy(apiKey = "revoked-replacement")))
        assertNotEquals(route, androidHttpModelRoute(config.copy(baseUrl = "https://example.invalid/v1?key=replaced")))
        assertTrue(runCatching { androidHttpModelRoute(config.copy(baseUrl = "https://user:secret@example.invalid/")) }.isFailure)
    }

    @Test fun frozenGenerationOverridesCurrentDefaultsAndRejectsDifferentOutputReservation() {
        val binding = buildJsonObject { put("generation", buildJsonObject {
            put("temperature", 0.25); put("top_p", 0.75); put("max_output_tokens", 1024)
            put("thinking_enabled", false); put("generation_options", buildJsonObject { put("top_k", 17) })
        }) }
        val context = buildJsonObject { put("context_window_tokens", 32768); put("reserved_output_tokens", 1024) }
        val parameters = frozenAndroidParameters(binding, context)
        assertEquals(0.25f, parameters.temperature)
        assertEquals(17, parameters.topK)
        assertEquals(32768, parameters.maxContextTokens)
        assertFalse(parameters.enableThinking)
        assertTrue(runCatching { frozenAndroidParameters(binding,
            JsonObject(context + ("reserved_output_tokens" to JsonPrimitive(2048)))) }.isFailure)
    }

    @Test fun policyIdentityIgnoresMapOrderingButNotPermissionChanges() {
        val model = buildJsonObject {
            put("primary", androidNativeModelRoute("fixture")); put("generation", buildJsonObject {})
            put("output_admission", buildJsonObject {}); put("model_capabilities", buildJsonObject {})
        }
        val config = buildJsonObject {
            put("host_catalog", buildJsonObject { put("b", 2); put("a", 1) })
            put("tools", JsonArray(emptyList())); put("permission_rules", JsonArray(emptyList()))
            put("context_window", buildJsonObject { put("reserved_output_tokens", 1024) })
        }
        val identity = bindAndroidInvocation(model, config)
        assertEquals(1, identity.getValue("version").jsonPrimitive.int)
        assertEquals(identity, bindAndroidInvocation(model, canonicalInvocationJson(config).jsonObject))
        assertNotEquals(identity, bindAndroidInvocation(model,
            JsonObject(config + ("permission_rules" to JsonArray(listOf(JsonPrimitive("changed")))))))
    }
}
