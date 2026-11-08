package com.nola.core

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidContextCoordinatorTest {
    private val budget = AgentCoreContextPrepareBudget(contextWindowTokens = 8_192)

    @Test
    fun everyProductionRouteSendsOnlyCoreCanonicalPayload() = runTest {
        AndroidContextRoute.entries.forEach { route ->
            val canonical = listOf(buildJsonObject {
                put("id", "core-$route")
                put("role", "user")
                put("content", "prepared")
            })
            val tools = listOf(buildJsonObject { put("name", "prepared_tool") })
            val coordinator = AndroidContextCoordinator { readyOutput(canonical, tools) }

            val prepared = coordinator.prepare(
                AndroidContextRequest(
                    route = route,
                    messages = listOf(buildJsonObject {
                        put("id", "raw")
                        put("role", "user")
                        put("content", "must not escape")
                    }),
                    components = listOf(
                        AgentCoreContextComponent("attachment", "attachment-1", JsonPrimitive("PDF body"))
                    ),
                    tools = listOf(buildJsonObject { put("name", "raw_tool") }),
                    budget = budget,
                )
            )

            assertEquals(canonical, prepared.modelInput.canonicalMessages)
            assertEquals(tools, prepared.modelInput.tools)
            assertEquals("payload", prepared.modelInput.contextPayloadDigest)
            assertTrue(prepared.modelInput.messages.isEmpty())
            assertEquals(null, prepared.modelInput.systemPrompt)
        }
    }

    @Test
    fun readOnlyPreparationCannotExecuteAnUnboundSummaryTransaction() = runTest {
        var calls = 0
        val coordinator = AndroidContextCoordinator { calls += 1; summaryRequiredOutput("b".repeat(64)) }
        val result = runCatching { coordinator.prepare(AndroidContextRequest(
            route = AndroidContextRoute.Cloud, messages = listOf(message("m1", "old")),
            components = emptyList(), tools = emptyList(), budget = budget,
        )) }
        assertTrue(result.exceptionOrNull() is AndroidContextPreparationException)
        assertEquals(1, calls)
    }

    @Test
    fun toolTransactionReachesProviderWithoutLosingCallOrResultFields() = runTest {
        val assistantCall = buildJsonObject {
            put("id", "assistant-1")
            put("role", "assistant")
            put("content", "")
            put(
                "tool_calls",
                kotlinx.serialization.json.JsonArray(
                    listOf(
                        buildJsonObject {
                            put("id", "call-1")
                            put("type", "function")
                            put(
                                "function",
                                buildJsonObject {
                                    put("name", "web_search")
                                    put("arguments", "{\"query\":\"2026 AI papers\"}")
                                },
                            )
                        },
                    ),
                ),
            )
        }
        val toolResult = buildJsonObject {
            put("id", "tool-1")
            put("role", "tool")
            put("tool_call_id", "call-1")
            put("name", "web_search")
            put("content", "result")
        }
        val canonical = listOf(message("m1", "search"), assistantCall, toolResult)
        val coordinator = AndroidContextCoordinator { readyOutput(canonical, emptyList()) }

        val prepared = coordinator.prepare(
            AndroidContextRequest(
                route = AndroidContextRoute.Cloud,
                messages = canonical,
                components = emptyList(),
                tools = emptyList(),
                budget = budget,
            ),
        )

        val sentMessages = prepared.modelInput.canonicalMessages
        assertEquals(canonical, sentMessages)
        assertEquals("call-1", sentMessages[2]["tool_call_id"]?.jsonPrimitive?.content)
        assertEquals(
            "web_search",
            sentMessages[1]["tool_calls"]
                ?.let { it as kotlinx.serialization.json.JsonArray }
                ?.first()
                ?.jsonObject
                ?.get("function")
                ?.jsonObject
                ?.get("name")
                ?.jsonPrimitive
                ?.content,
        )
    }

    @Test
    fun meterUsesOnlyCoreMetricsAndComponentMetrics() {
        val output = readyOutput(listOf(message("m1", "hello")), emptyList()).copy(
            metrics = metrics(messageTokens = 120, toolTokens = 10, total = 130),
            componentMetrics = listOf(
                AgentCoreContextComponentMetrics("session_memory", 20, 0, 0),
                AgentCoreContextComponentMetrics("memory", 7, 0, 0),
                AgentCoreContextComponentMetrics("personal_intelligence", 3, 0, 0),
                AgentCoreContextComponentMetrics("document", 30, 0, 0),
            ),
        )

        val usage = output.toContextUsage()

        assertEquals(130, usage.totalTokens)
        assertEquals(60, usage.breakdown.messages)
        assertEquals(20, usage.breakdown.sessionMemory)
        assertEquals(10, usage.breakdown.longTermMemory)
        assertEquals(30, usage.breakdown.uploadedDocuments)
        assertEquals(10, usage.breakdown.mcpTools)
        assertTrue(usage.suggestions.isEmpty())
    }

    private fun message(id: String, content: String) = buildJsonObject {
        put("id", id)
        put("role", "user")
        put("content", content)
    }

    private fun readyOutput(messages: List<kotlinx.serialization.json.JsonObject>, tools: List<kotlinx.serialization.json.JsonElement>) =
        AgentCoreContextPrepareOutput(
            schemaVersion = 1,
            action = "ready",
            messages = messages,
            tools = tools,
            payloadDigest = "payload",
            metrics = metrics(),
        )

    private fun summaryRequiredOutput(digest: String) = AgentCoreContextPrepareOutput(
        schemaVersion = 1,
        action = "summary_required",
        messages = listOf(message("m2", "new")),
        payloadDigest = "summary-payload",
        metrics = metrics(),
        archiveRequest = AgentCoreContextArchiveRequest(
            archiveId = "archive-1",
            archiveDigest = digest,
            messageIndices = listOf(0),
            messageIds = listOf("m1"),
            messages = listOf(message("m1", "old")),
            targetSummaryTokens = 128,
        ),
    )

    private fun cannotFitOutput(reason: String) = AgentCoreContextPrepareOutput(
        schemaVersion = 1,
        action = "cannot_fit",
        messages = emptyList(),
        payloadDigest = "cannot-fit",
        reason = reason,
        metrics = metrics(),
    )

    private fun metrics(messageTokens: Int = 10, toolTokens: Int = 0, total: Int = 10) =
        AgentCoreContextMetrics(
            messageTokens = messageTokens,
            toolSchemaTokens = toolTokens,
            totalPromptTokens = total,
            promptTokenLimit = 8_000,
            reservedOutputTokens = 192,
            usagePercent = total.toDouble() / 80.0,
        )
}
