package com.nola.ui.screen.models

import com.nola.data.repository.ApiModelConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelInferenceTestConfigurationTest {
    @Test
    fun testUsesRequestedProfileWireModelAndSavedParameters() {
        val endpoint = ApiModelConfig("https://example.test/v1", "test-key", "selected-model")
        val model = ModelInfo(
            id = "model-to-test",
            name = "Model to test",
            profileId = "profile-to-test",
        )
        val parameters = ModelParameters(
            temperature = 0.25f,
            topP = 0.8f,
            topK = 12,
            maxCompletionTokens = 333,
        )

        val configuration = modelTestCloudConfiguration(endpoint, model, parameters)

        assertEquals("https://example.test/v1", configuration.baseUrl)
        assertEquals("test-key", configuration.apiKey)
        assertEquals("model-to-test", configuration.model)
        assertEquals(0.25f, configuration.temperature)
        assertEquals(0.8f, configuration.topP)
        assertEquals(12, configuration.topK)
        assertEquals(333, configuration.maxCompletionTokens)
        assertTrue(configuration.sendTopK)
        assertEquals("selected-model", endpoint.model)
    }
}
