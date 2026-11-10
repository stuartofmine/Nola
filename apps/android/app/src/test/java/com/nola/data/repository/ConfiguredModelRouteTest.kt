package com.nola.data.repository

import com.nola.data.local.CanonicalModelProfile
import com.nola.domain.model.ModelWireProtocol
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import org.junit.Test

class ConfiguredModelRouteTest {
    private fun profile(sourceJson: String) = CanonicalModelProfile(
        id = "profile", displayName = "Model", provider = "openai-compatible",
        modelName = "model", runtime = "remote", sourceJson = sourceJson,
        parametersJson = "{}", selected = false, enabled = true, revision = 1,
        createdAtMs = 1, updatedAtMs = 1,
    )

    @Test
    fun executionCapabilitiesDefaultToolsOnAndRespectExactConfiguredModelOverrides() {
        val declared = profile("""{"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}""")
            .toConfiguredModelInfo()
        assertTrue(declared.executionCapabilities.toolCalls)
        assertTrue(declared.executionCapabilities.samplingParameters)
        assertEquals(1L, declared.profileRevision)
        val unknown = profile("{}").toConfiguredModelInfo()
        assertTrue(unknown.executionCapabilities.toolCalls)
        assertFalse(unknown.executionCapabilities.samplingParameters)
        val partial = profile("""{"executionCapabilities":{"imageInput":true}}""").toConfiguredModelInfo()
        assertTrue(partial.executionCapabilities.toolCalls)
        assertTrue(partial.executionCapabilities.imageInput)
        val disabled = profile("""{"executionCapabilities":{"toolCalls":false}}""").toConfiguredModelInfo()
        assertFalse(disabled.executionCapabilities.toolCalls)
        val invalid = profile("""{"executionCapabilities":{"toolCalls":"invalid"}}""").toConfiguredModelInfo()
        assertFalse(invalid.executionCapabilities.toolCalls)
    }

}
