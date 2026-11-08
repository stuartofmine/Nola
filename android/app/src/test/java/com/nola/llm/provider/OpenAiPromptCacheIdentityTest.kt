package com.nola.llm.provider

import com.nola.domain.model.ModelInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OpenAiPromptCacheIdentityTest {
    private val digest = "ab".repeat(32)

    @Test
    fun mainTurnUsesExplicitKeyBeforeDerivedIdentity() {
        val config = config(
            promptCacheKey = "configured-main-turn-key",
            promptCacheKeyPrefix = "ignored-prefix",
        )

        assertEquals(
            "configured-main-turn-key",
            promptCacheKeyForRequest(ModelInput(stablePromptDigest = digest), config),
        )
    }

    @Test
    fun derivedIdentityUsesOnlyStableDigestAndDeterministicNamespace() {
        val input = ModelInput(
            systemPrompt = "dynamic rendered prompt must not define cache identity",
            stablePromptDigest = digest,
        )
        val first = promptCacheKeyForRequest(input, config(promptCacheKeyPrefix = "nola"))
        val second = promptCacheKeyForRequest(
            input.copy(systemPrompt = "different rendered prompt"),
            config(promptCacheKeyPrefix = "nola"),
        )

        assertEquals(first, second)
        assertTrue(first?.matches(Regex("[0-9a-f]{64}")) == true)
        assertNotEquals(first, promptCacheKeyForRequest(input, config(model = "other-model")))
        assertNotEquals(
            first,
            promptCacheKeyForRequest(input, config(baseUrl = "https://other.example/v1")),
        )
        assertNotEquals(first, promptCacheKeyForRequest(input, config(promptCacheKeyPrefix = "other")))
    }

    @Test
    fun auxiliaryAndUnsupportedRequestsDoNotSendCacheIdentity() {
        assertNull(promptCacheKeyForRequest(ModelInput(), config(promptCacheKey = "static-key")))
        assertNull(
            promptCacheKeyForRequest(
                ModelInput(stablePromptDigest = digest),
                config(supportsPromptCacheKey = false, promptCacheKey = "static-key"),
            ),
        )
        assertNull(
            promptCacheKeyForRequest(
                ModelInput(stablePromptDigest = "not-a-core-digest"),
                config(),
            ),
        )
    }

    private fun config(
        baseUrl: String = "https://api.openai.com/v1",
        model: String = "gpt-test",
        supportsPromptCacheKey: Boolean = true,
        promptCacheKey: String? = null,
        promptCacheKeyPrefix: String? = null,
    ) = OpenAiProvider.Config(
        baseUrl = baseUrl,
        apiKey = "test",
        model = model,
        supportsPromptCacheKey = supportsPromptCacheKey,
        promptCacheKey = promptCacheKey,
        promptCacheKeyPrefix = promptCacheKeyPrefix,
    )
}
