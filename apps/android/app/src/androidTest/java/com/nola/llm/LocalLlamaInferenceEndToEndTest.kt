package com.nola.llm

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.arm.aichat.AiChat
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelParameters
import com.nola.llm.local.DeviceModelRecord
import com.nola.llm.local.DeviceModelStore
import com.nola.llm.provider.LocalLlamaProvider
import java.io.File
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class LocalLlamaInferenceEndToEndTest {
    @Test
    fun loadsRealGgufAndGeneratesTextThroughNativeRuntime() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val model = File(context.filesDir, "models/e2e-qwen3-0.6b.gguf")
        assertTrue("Real GGUF fixture was not provisioned at ${model.absolutePath}", model.isFile)
        assertTrue("GGUF fixture is unexpectedly small", model.length() > 400_000_000)

        val engine = AiChat.getInferenceEngine(context)
        try {
            withTimeout(120_000) {
                engine.loadModel(model.absolutePath)
                engine.setSystemPrompt("You are Nola. Answer concisely.")
            }
            val chunks = withTimeout(120_000) {
                engine.sendUserPrompt("Reply with exactly: local model works", predictLength = 24)
                    .take(24)
                    .toList()
            }
            assertTrue("Native inference returned no text", chunks.joinToString("").isNotBlank())
        } finally {
            engine.cleanUp()
        }
    }

    @Test
    fun explicitProviderLifecycleStreamsFromRealModel() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val file = File(context.filesDir, "models/e2e-qwen3-0.6b.gguf")
        val record = DeviceModelRecord(
            id = "e2e-qwen3-0.6b",
            displayName = "Qwen3 0.6B E2E",
            fileName = file.name,
            sourceUrl = "e2e-fixture",
            sha256 = "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4",
            byteSize = file.length(),
            contextWindow = 65_536,
            recommendedRamBytes = 2L * 1024 * 1024 * 1024,
            installedAt = "e2e",
        )
        val provider = LocalLlamaProvider(context, DeviceModelStore(context))
        try {
            provider.start(
                record,
                ModelParameters(
                    temperature = 0.2f,
                    topP = 0.8f,
                    topK = 12,
                    maxCompletionTokens = 24,
                    maxContextTokens = 8192,
                ),
            )
            val output = withTimeout(120_000) {
                provider.generateSummary(
                    ModelInput(messages = listOf(mapOf("role" to "user", "content" to "Say hello."))),
                ).text
            }
            assertTrue("Provider stream returned no text", output.isNotBlank())
        } finally {
            provider.stop()
        }
    }
}
