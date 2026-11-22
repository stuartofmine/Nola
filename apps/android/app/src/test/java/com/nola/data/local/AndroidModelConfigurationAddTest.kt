package com.nola.data.local

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidModelConfigurationAddTest {
    private fun profile(
        id: String,
        modelName: String,
        endpoint: String,
        endpointId: String,
        selected: Boolean = false,
        enabled: Boolean = true,
        endpointOrder: Int = 0,
        modelOrder: Int = 0,
        parametersJson: String = "{}",
        revision: Long = 1L,
    ): CanonicalModelProfile = CanonicalModelProfile(
        id = id,
        displayName = modelName,
        provider = "openai-compatible",
        modelName = modelName,
        runtime = "remote",
        sourceJson = """{"schemaVersion":2,"endpointID":"$endpointId","endpointName":"Host","baseURL":"$endpoint","endpointEnabled":true,"endpointOrder":$endpointOrder,"modelEnabled":$enabled,"modelOrder":$modelOrder,"priority":50,"capabilities":["chat"],"wireProtocol":"chat_completions"}""",
        parametersJson = parametersJson,
        selected = selected,
        enabled = enabled,
        revision = revision,
        createdAtMs = 100L,
        updatedAtMs = 100L,
        deletedAtMs = null,
    )

    @Test
    fun addingSecondModelOnSameEndpointPreservesFirstModel() {
        val existing = listOf(
            profile("gpt-4o", "gpt-4o", "https://api.openai.com/v1", "ep-1", selected = true, modelOrder = 0)
        )

        val merged = mergeAddedRemoteModelProfiles(
            catalog = existing,
            normalizedEndpoint = "https://api.openai.com/v1",
            names = listOf("gpt-4o-mini"),
            wireProtocol = "chat_completions",
            credentialReference = "android-model:cred-2",
            now = 200L,
        )

        assertEquals(2, merged.size)
        val first = merged.single { it.modelName == "gpt-4o" }
        val second = merged.single { it.modelName == "gpt-4o-mini" }
        assertTrue(first.enabled)
        assertFalse(first.selected)
        assertTrue(second.enabled)
        assertTrue(second.selected)
        assertTrue(second.sourceJson.contains("\"endpointID\":\"ep-1\""))
    }

    @Test
    fun addingModelOnNewEndpointPreservesExistingEndpointGroup() {
        val existing = listOf(
            profile("gpt-4o", "gpt-4o", "https://api.openai.com/v1", "ep-1", selected = true, endpointOrder = 0, modelOrder = 0),
            profile("gpt-4o-mini", "gpt-4o-mini", "https://api.openai.com/v1", "ep-1", selected = false, endpointOrder = 0, modelOrder = 1),
        )

        val merged = mergeAddedRemoteModelProfiles(
            catalog = existing,
            normalizedEndpoint = "https://openrouter.ai/api/v1",
            names = listOf("claude-3.7-sonnet"),
            wireProtocol = "responses",
            credentialReference = "android-model:cred-3",
            now = 300L,
        )

        assertEquals(3, merged.size)
        assertEquals(setOf("gpt-4o", "gpt-4o-mini", "claude-3.7-sonnet"), merged.map { it.modelName }.toSet())
        assertEquals(1, merged.count { it.selected })
        assertTrue(merged.single { it.modelName == "claude-3.7-sonnet" }.selected)
        assertTrue(merged.single { it.modelName == "claude-3.7-sonnet" }.sourceJson.contains("\"endpointOrder\":1"))
    }

    @Test
    fun reAddingExistingModelUpdatesInPlaceWithoutDeletingSiblings() {
        val existing = listOf(
            profile("id-a", "gpt-4o", "https://api.openai.com/v1", "ep-1", selected = true, parametersJson = """{"temperature":0.2}""", revision = 7L),
            profile("id-b", "gpt-4o-mini", "https://api.openai.com/v1", "ep-1", selected = false, modelOrder = 1),
        )

        val merged = mergeAddedRemoteModelProfiles(
            catalog = existing,
            normalizedEndpoint = "https://api.openai.com/v1",
            names = listOf("gpt-4o"),
            wireProtocol = "responses",
            credentialReference = null,
            now = 400L,
        )

        assertEquals(2, merged.size)
        val updated = merged.single { it.modelName == "gpt-4o" }
        assertEquals("id-a", updated.id)
        assertEquals("""{"temperature":0.2}""", updated.parametersJson)
        assertEquals(7L, updated.revision)
        assertTrue(updated.sourceJson.contains("\"wireProtocol\":\"responses\""))
        assertTrue(merged.any { it.id == "id-b" && it.enabled })
    }
}
