package com.nola.core

import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Test

class AgentPromptComposerTest {
    @Test
    fun abiCodecUsesCoreV1LayerAndTrustNames() {
        val bytes = AgentPromptComposerCodec.encode(
            AgentPromptComposeInput(
                fragments = listOf(
                    AgentPromptFragment(
                        id = "workflow",
                        layer = AgentPromptLayer.ExecutionPolicy,
                        trust = AgentPromptTrust.HostInstruction,
                        cacheable = true,
                        content = "Policy",
                    ),
                    AgentPromptFragment(
                        id = "reference",
                        layer = AgentPromptLayer.ConversationContext,
                        trust = AgentPromptTrust.Reference,
                        cacheable = false,
                        content = "Document",
                    ),
                    AgentPromptFragment(
                        id = "runtime",
                        layer = AgentPromptLayer.RuntimeState,
                        trust = AgentPromptTrust.HostFact,
                        cacheable = false,
                        content = "Current workspace",
                    ),
                ),
            ),
        )
        val root = kotlinx.serialization.json.Json.parseToJsonElement(bytes.decodeToString()).jsonObject
        assertEquals(1, root["schema_version"]?.jsonPrimitive?.content?.toInt())
        assertEquals("execution_policy", root["fragments"]?.jsonArray?.get(0)?.jsonObject?.get("layer")?.jsonPrimitive?.content)
        assertEquals("host_instruction", root["fragments"]?.jsonArray?.get(0)?.jsonObject?.get("trust")?.jsonPrimitive?.content)
        assertEquals("reference", root["fragments"]?.jsonArray?.get(1)?.jsonObject?.get("trust")?.jsonPrimitive?.content)
        assertFalse(root["fragments"]?.jsonArray?.get(1)?.jsonObject?.get("cacheable")?.jsonPrimitive?.content.toBoolean())
        assertEquals("runtime_state", root["fragments"]?.jsonArray?.get(2)?.jsonObject?.get("layer")?.jsonPrimitive?.content)
    }

    @Test
    fun mainTurnPreservesCoreMessagesWithoutHostRecomposition() {
        val composer = AgentPromptComposer { _ ->
            error("Main Agent turns must not be recomposed by the Android Host")
        }
        val query = "UNIQUE USER QUERY"
        val request = AgentModelRequest(
            sessionId = "session",
            turnId = "turn",
            iteration = 2,
            messages = listOf(
                AgentCoreMessage(role = "system", content = "retrieved context"),
                AgentCoreMessage(role = "assistant", content = "earlier answer"),
                AgentCoreMessage(role = "user", content = query),
            ),
            tools = listOf(buildJsonObject {
                put("type", "function")
                put("function", buildJsonObject {
                    put("name", "device_read_file")
                    put("description", "Read a file")
                })
            }),
            reservedOutputTokens = 1024,
            stablePromptDigest = "core-digest",
            invocationPlan = AgentModelInvocationPlan(
                nativeToolCalls = true,
                promptCache = true,
            ),
        )

        val envelope = AgentRequestModelBuilder(composer).build(
            request = request,
            enableThinking = true,
        )

        assertEquals("system", envelope.modelInput.canonicalMessages.first()["role"]?.jsonPrimitive?.content)
        assertEquals("retrieved context", envelope.modelInput.canonicalMessages.first()["content"]?.jsonPrimitive?.content)
        assertEquals("user", envelope.modelInput.canonicalMessages.last()["role"]?.jsonPrimitive?.content)
        assertEquals(query, envelope.modelInput.canonicalMessages.last()["content"]?.jsonPrimitive?.content)
        assertEquals(listOf("device_read_file"), envelope.allowedToolNames)
        assertEquals(request.tools, envelope.modelInput.tools)
        assertEquals("core-digest", envelope.modelInput.stablePromptDigest)
        assertEquals(null, envelope.composition)
    }

    @Test
    fun auxiliaryRequestComposesOnlyExplicitPolicyFragments() {
        var captured = emptyList<AgentPromptFragment>()
        val composer = AgentPromptComposer { fragments ->
            captured = fragments
            AgentPromptComposition("", "AUXILIARY", "AUXILIARY", fragments.map { it.id }, "aux")
        }
        val policy = AgentPromptFragment(
            id = "core-context-compaction",
            layer = AgentPromptLayer.ExecutionPolicy,
            trust = AgentPromptTrust.HostInstruction,
            cacheable = false,
            content = "Summarize faithfully.",
        )
        val request = AgentModelRequest(
            sessionId = "session",
            turnId = "compaction",
            iteration = 1,
            messages = listOf(AgentCoreMessage(role = "user", content = "history")),
            tools = emptyList(),
            reservedOutputTokens = 1024,
            policyFragments = listOf(policy),
        )

        val envelope = AgentRequestModelBuilder(composer).build(request, enableThinking = false)

        assertEquals(listOf("core-context-compaction"), captured.map { it.id })
        assertEquals("AUXILIARY", envelope.modelInput.canonicalMessages.first()["content"]?.jsonPrimitive?.content)
        assertEquals(null, envelope.modelInput.stablePromptDigest)
    }

    @Test
    fun requiredToolChoiceIsTypedAndMustExistInTheRequestCatalog() {
        val tool = buildJsonObject {
            put("type", "function")
            put("function", buildJsonObject {
                put("name", "submit_memory_worker_output")
                put("parameters", buildJsonObject { put("type", "object") })
            })
        }
        val request = AgentModelRequest(
            sessionId = "memory-worker",
            turnId = "memory-job",
            iteration = 1,
            messages = listOf(AgentCoreMessage("user", "{}")),
            tools = listOf(tool),
            reservedOutputTokens = 1024,
            invocationPlan = AgentModelInvocationPlan(nativeToolCalls = true),
            requiredToolName = "submit_memory_worker_output",
        )

        val envelope = AgentRequestModelBuilder().build(request, enableThinking = false)

        assertEquals(
            "submit_memory_worker_output",
            envelope.modelInput.userPrefs["required_tool_name"],
        )
        assertThrows(IllegalArgumentException::class.java) {
            AgentRequestModelBuilder().build(
                request.copy(requiredToolName = "missing_tool"),
                enableThinking = false,
            )
        }
    }

    @Test
    fun abiCodecDecodesCoreCompositionAndErrors() {
        val success = AgentPromptComposerCodec.decode(
            """{"schema_version":1,"composition":{"stable_prefix":"a","dynamic_suffix":"b","rendered":"a\n\nb","fragment_ids":["one"],"stable_prefix_sha256":"hash","stable_fragment_sha256":{"one":"fragment-hash"}}}""".encodeToByteArray(),
        )
        assertEquals("a\n\nb", success.composition?.rendered)
        assertEquals(mapOf("one" to "fragment-hash"), success.composition?.stableFragmentSha256)

        val failure = AgentPromptComposerCodec.decode(
            """{"schema_version":1,"error":{"code":"duplicate_id","message":"one"}}""".encodeToByteArray(),
        )
        assertEquals("duplicate_id", failure.error?.code)
    }
}
