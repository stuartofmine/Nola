package com.nola.core

import com.nola.domain.model.ContextBreakdown
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.ModelInput
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

enum class AndroidContextRoute {
    Cloud,
    Web,
    Local,
    PermissionResume,
    DeviceResume,
}

class AndroidContextPreparationException(message: String) : IllegalStateException(message)

data class AndroidContextRequest(
    val route: AndroidContextRoute,
    val messages: List<JsonObject>,
    val components: List<AgentCoreContextComponent>,
    val tools: List<JsonElement>,
    val budget: AgentCoreContextPrepareBudget,
    val mode: String = "normal",
    val existingSummary: String? = null,
    val userPrefs: Map<String, String> = emptyMap(),
)

data class AndroidPreparedContext(
    val route: AndroidContextRoute,
    val modelInput: ModelInput,
    val output: AgentCoreContextPrepareOutput,
)

class AndroidContextCoordinator(
    private val corePrepare: (AgentCoreContextPrepareInput) -> AgentCoreContextPrepareOutput? =
        AgentCoreContextPreparation::prepare,
) {
    suspend fun prepare(
        request: AndroidContextRequest,
    ): AndroidPreparedContext {
        val baseInput = AgentCoreContextPrepareInput(
            mode = request.mode,
            components = request.components,
            messages = request.messages,
            tools = request.tools,
            budget = request.budget,
            existingSummary = request.existingSummary,
        )
        val finalOutput = prepareOrThrow(baseInput)
        if (finalOutput.action != "ready") {
            throw AndroidContextPreparationException("Context maintenance requires its Application transaction: ${finalOutput.reason}")
        }
        if (finalOutput.schemaVersion != AGENT_CORE_CONTEXT_SCHEMA_VERSION) {
            throw AndroidContextPreparationException("Unexpected Core context schema ${finalOutput.schemaVersion}")
        }
        if (finalOutput.payloadDigest.isBlank()) {
            throw AndroidContextPreparationException("Core returned a payload without a digest")
        }
        return AndroidPreparedContext(
            route = request.route,
            modelInput = ModelInput(
                userPrefs = request.userPrefs,
                canonicalMessages = finalOutput.messages,
                tools = finalOutput.tools,
                contextPayloadDigest = finalOutput.payloadDigest,
            ),
            output = finalOutput,
        )
    }

    private fun prepareOrThrow(input: AgentCoreContextPrepareInput): AgentCoreContextPrepareOutput =
        corePrepare(input)
            ?: throw AndroidContextPreparationException("Nola Core context engine ABI 2 is unavailable")
}

fun AgentCoreContextPrepareOutput.toContextUsage(): ContextUsage {
    fun used(vararg kinds: String): Int = componentMetrics
        .filter { it.kind in kinds }
        .sumOf { it.usedTokens }

    val componentTokens = componentMetrics.sumOf { it.usedTokens }
    return ContextUsage(
        totalTokens = metrics.totalPromptTokens,
        maxTokens = metrics.promptTokenLimit,
        usagePercent = metrics.usagePercent.toFloat(),
        breakdown = ContextBreakdown(
            messages = (metrics.messageTokens - componentTokens).coerceAtLeast(0),
            sessionMemory = used("session_memory"),
            longTermMemory = used("memory", "personal_intelligence"),
            userPreferences = used("user_preference"),
            mcpTools = metrics.toolSchemaTokens + used("mcp"),
            uploadedSkills = used("skill"),
            uploadedDocuments = used("document"),
            dataSources = used("attachment", "web_retrieval", "platform_data"),
            scaffold = used("system", "prompt_template"),
        ),
        suggestions = emptyList(),
    )
}
