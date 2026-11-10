package com.nola.core

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.toList
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID

/** Real JNI plan/decision integration; model completion is an explicit test double. */
@RunWith(AndroidJUnit4::class)
class CompactionSummaryPlanNativeTest {
    @Test fun nonStreamingAgentRetainsProviderOutputLimit() = runBlocking {
        val provider = object : com.nola.llm.LlmClient {
            override suspend fun generateSummary(input: com.nola.domain.model.ModelInput, timeout: kotlin.time.Duration?) =
                com.nola.domain.model.ModelOutput("Partial checkpoint", finishReason = "length")
            override fun generateSummaryStream(input: com.nola.domain.model.ModelInput) = kotlinx.coroutines.flow.emptyFlow<String>()
            override suspend fun chat(message: String, systemPrompt: String?, enableThinking: Boolean, timeout: kotlin.time.Duration?) = ""
        }
        val result = TextAgentModelClient(provider).generateAgentTurn(AgentModelRequest(
            sessionId = "metadata", turnId = "metadata-turn", iteration = 1,
            messages = listOf(AgentCoreMessage("user", "summarize")), tools = emptyList(), reservedOutputTokens = 100,
        )).toList().filterIsInstance<AgentModelEvent.Completed>().single().completion
        assertEquals("length", result.finishReason)
    }

    @Test fun coldArchiveRunsMultipleCoreSegmentsBeforeReturningFinalCheckpoint() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val directory = File(context.cacheDir, "summary-plan-${UUID.randomUUID()}")
        val database = File(directory, "product.db")
        AndroidApplicationRuntime(context, database).use { application ->
            var calls = 0
            val executor = CompactionSummaryExecutor(
                execute = { application.execute(it).jsonObject },
                generate = { messages, instructions, _ ->
                    calls += 1
                    assertTrue(instructions.contains("checkpoint"))
                    assertTrue(messages.isNotEmpty())
                    AgentModelCompletion(content = "# Checkpoint\n- Preserve requirement ALPHA and source references.", finishReason = "stop")
                },
            )
            val archive = (1..10).map { index -> buildJsonObject {
                put("role", "user"); put("content", "Source $index ALPHA " + "Long historical evidence. ".repeat(180))
            } }
            val result = executor.summarize(archive, 16000, 2000, 500, "Preserve user requirements and evidence.")
            assertTrue("A cold archive must execute multiple bounded Core segments", calls > 1)
            assertTrue(result.content.contains("ALPHA"))
        }
        directory.deleteRecursively()
        Unit
    }
}
