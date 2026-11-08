package com.nola.core

import com.nola.llm.LlmRequestException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.serialization.json.*

/** Executes opaque Core plan steps. Segment boundaries and acceptance remain in Core. */
internal class CompactionSummaryExecutor(
    private val execute: (JsonObject) -> JsonObject,
    private val generate: suspend (List<AgentCoreMessage>, String, JsonObject?) -> AgentModelCompletion,
) {
    suspend fun summarize(messages: List<JsonElement>, window: Int, output: Int, target: Int,
                          instructions: String): AgentModelCompletion {
        fun step(fields: JsonObject) = execute(buildJsonObject {
            put("type", "step_compaction_summary"); put("step", fields)
        })
        var plan = step(buildJsonObject {
            put("type", "start"); put("messages", JsonArray(messages))
            put("context_window_tokens", window); put("output_tokens", output)
            put("target_tokens", target); put("instructions", instructions)
        })
        var last = AgentModelCompletion(content = "")
        while (true) {
            currentCoroutineContext().ensureActive()
            when (plan.getValue("action").jsonPrimitive.content) {
                "complete" -> return last.copy(content = plan.getValue("summary").jsonPrimitive.content)
                "request" -> Unit
                else -> error("Invalid Core compaction plan step")
            }
            val opaqueState = plan.getValue("state")
            val source = plan.getValue("messages").jsonArray.map {
                val item = it.jsonObject
                AgentCoreMessage(role = item.getValue("role").jsonPrimitive.content,
                    content = item.getValue("content").jsonPrimitive.content)
            }
            val baseInstructions = source.filter { it.role == "system" }.joinToString("\n\n") { it.content }
            val baseMessages = source.filter { it.role != "system" }
            val segmentTarget = plan.getValue("target_tokens").jsonPrimitive.int
            var currentMessages = baseMessages
            var currentInstructions = baseInstructions
            var continuation: JsonObject? = null
            var partial = ""
            var continues = 0
            var rewrites = 0
            segment@ while (true) {
                currentCoroutineContext().ensureActive()
                val result = try { generate(currentMessages, currentInstructions, continuation) }
                catch (error: LlmRequestException) {
                    if (error.code != "context_length_exceeded") throw error
                    plan = step(buildJsonObject { put("type", "split"); put("state", opaqueState) })
                    break@segment
                }
                last = result
                val summary = partial + result.content
                val decision = execute(buildJsonObject {
                    put("type", "evaluate_compaction_maintenance"); put("facts", buildJsonObject {
                        put("summary", summary); put("target_tokens", segmentTarget)
                        put("output_limited", result.finishReason in setOf("length", "max_tokens", "max_output_tokens", "token_limit"))
                        put("has_tool_calls", result.toolCalls.isNotEmpty())
                        put("completed", result.finishReason in setOf("stop", "end_turn", "completed"))
                        put("cancelled", false); put("continuation_count", continues); put("rewrite_count", rewrites)
                        put("can_continue", result.providerContinuation != null); put("can_rewrite", true)
                    })
                })
                currentCoroutineContext().ensureActive()
                when (decision.getValue("action").jsonPrimitive.content) {
                    "accept" -> {
                        plan = step(buildJsonObject { put("type", "accept"); put("state", opaqueState); put("summary", summary) })
                        break@segment
                    }
                    "continue", "rewrite" -> {
                        currentInstructions = baseInstructions + "\n\n" + decision.getValue("instructions").jsonPrimitive.content
                        currentMessages = baseMessages + AgentCoreMessage(role = "assistant", content = summary)
                        if (decision.getValue("action").jsonPrimitive.content == "continue") {
                            partial = summary; continuation = result.providerContinuation; continues += 1
                        } else { partial = ""; continuation = null; rewrites += 1 }
                    }
                    else -> error("Core rejected the context checkpoint; original context retained")
                }
            }
        }
    }
}
