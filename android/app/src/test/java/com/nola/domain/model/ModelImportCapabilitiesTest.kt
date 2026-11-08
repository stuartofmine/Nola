package com.nola.domain.model

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelImportCapabilitiesTest {
    @Test fun newModelDefaultsToToolCalls() {
        assertTrue(ModelInfo(id = "new", name = "new").executionCapabilities.toolCalls)
    }
    @Test fun explicitProviderTextOnlyOverridesDefault() {
        val provider = ModelExecutionCapabilities(toolCalls = false)
        assertFalse(ModelInfo(id = "text", name = "text", executionCapabilities = provider).executionCapabilities.toolCalls)
    }
}
