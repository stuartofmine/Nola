@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import com.nola.domain.model.ModelInput
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

const val AGENT_PROMPT_COMPOSER_SCHEMA_VERSION = 1

@Serializable
enum class AgentPromptLayer {
    @SerialName("identity") Identity,
    @SerialName("safety") Safety,
    @SerialName("tool_policy") ToolPolicy,
    @SerialName("execution_policy") ExecutionPolicy,
    @SerialName("memory_policy") MemoryPolicy,
    @SerialName("host_capabilities") HostCapabilities,
    @SerialName("runtime_state") RuntimeState,
    @SerialName("skill_context") SkillContext,
    @SerialName("conversation_context") ConversationContext,
    @SerialName("response_style") ResponseStyle,
}

@Serializable
enum class AgentPromptTrust {
    @SerialName("host_fact") HostFact,
    @SerialName("host_instruction") HostInstruction,
    @SerialName("user_instruction") UserInstruction,
    @SerialName("reference") Reference,
}

@Serializable
data class AgentPromptFragment(
    val id: String,
    val layer: AgentPromptLayer,
    val trust: AgentPromptTrust,
    val cacheable: Boolean,
    val content: String,
)

@Serializable
data class AgentPromptComposeInput(
    @SerialName("schema_version") val schemaVersion: Int = AGENT_PROMPT_COMPOSER_SCHEMA_VERSION,
    val fragments: List<AgentPromptFragment>,
)

@Serializable
data class AgentPromptComposition(
    @SerialName("stable_prefix") val stablePrefix: String,
    @SerialName("dynamic_suffix") val dynamicSuffix: String,
    val rendered: String,
    @SerialName("fragment_ids") val fragmentIds: List<String>,
    @SerialName("stable_prefix_sha256") val stablePrefixSha256: String,
    @SerialName("stable_fragment_sha256") val stableFragmentSha256: Map<String, String> = emptyMap(),
)

@Serializable
data class AgentPromptComposeError(val code: String, val message: String)

@Serializable
data class AgentPromptComposeResponse(
    @SerialName("schema_version") val schemaVersion: Int,
    val composition: AgentPromptComposition? = null,
    val error: AgentPromptComposeError? = null,
)

internal object AgentPromptComposerCodec {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    fun encode(input: AgentPromptComposeInput): ByteArray =
        json.encodeToString(input).encodeToByteArray()

    fun decode(bytes: ByteArray): AgentPromptComposeResponse =
        json.decodeFromString(bytes.decodeToString())
}

fun interface AgentPromptComposer {
    fun compose(fragments: List<AgentPromptFragment>): AgentPromptComposition
}

object CoreAgentPromptComposer : AgentPromptComposer {
    override fun compose(fragments: List<AgentPromptFragment>): AgentPromptComposition {
        check(AgentCoreNative.isPromptComposerAvailable) {
            "Nola Core prompt composer ABI 1 is unavailable"
        }
        val bytes = AgentCoreNative.nativeComposeAgentPrompt(
            AgentPromptComposerCodec.encode(AgentPromptComposeInput(fragments = fragments)),
        ) ?: error("Nola Core prompt composer returned no response")
        val response = AgentPromptComposerCodec.decode(bytes)
        check(response.schemaVersion == AGENT_PROMPT_COMPOSER_SCHEMA_VERSION) {
            "Unexpected Nola Core prompt composer schema ${response.schemaVersion}"
        }
        response.error?.let { error("Nola Core prompt composition failed (${it.code}): ${it.message}") }
        return response.composition ?: error("Nola Core prompt composer returned no composition")
    }
}

data class AgentProviderRequestEnvelope(
    val modelInput: ModelInput,
    val composition: AgentPromptComposition?,
    val allowedToolNames: List<String>,
)

class AgentRequestModelBuilder(
    private val composer: AgentPromptComposer = CoreAgentPromptComposer,
) {
    fun build(
        request: AgentModelRequest,
        enableThinking: Boolean,
    ): AgentProviderRequestEnvelope {
        val composition = if (request.policyFragments.isEmpty()) {
            null
        } else {
            composer.compose(request.policyFragments)
        }
        val canonical = buildList {
            composition?.rendered?.takeIf(String::isNotBlank)?.let { policy ->
                add(buildJsonObject {
                    put("role", "system")
                    put("content", policy)
                })
            }
            addAll(request.messages.map(::providerMessage))
        }
        val nativeTools = if (request.invocationPlan.nativeToolCalls) request.tools else emptyList()
        val allowedToolNames = toolNames(nativeTools)
        request.requiredToolName?.let { requiredToolName ->
            require(requiredToolName in allowedToolNames) {
                "The required tool is not available in this model request"
            }
        }
        val userPrefs = buildMap {
            put("enable_thinking", (enableThinking && request.invocationPlan.reasoning).toString())
            put("parallel_tool_calls", request.invocationPlan.parallelToolCalls.toString())
            request.requiredToolName?.let { put("required_tool_name", it) }
        }
        return AgentProviderRequestEnvelope(
            modelInput = ModelInput(
                canonicalMessages = canonical,
                tools = nativeTools,
                userPrefs = userPrefs,
                stablePromptDigest = request.stablePromptDigest.takeIf {
                    request.invocationPlan.promptCache
                },
                providerContinuation = request.providerContinuation,
            ),
            composition = composition,
            allowedToolNames = allowedToolNames,
        )
    }

    private fun providerMessage(message: AgentCoreMessage): JsonObject = buildJsonObject {
        put("role", message.role)
        put("content", message.content)
        message.name?.let { put("name", it) }
        message.toolCallId?.let { put("tool_call_id", it) }
        if (message.toolCalls.isNotEmpty()) put("tool_calls", JsonArray(message.toolCalls))
    }

    private fun toolNames(tools: List<JsonElement>): List<String> = tools.mapNotNull { tool ->
        runCatching {
            tool.jsonObject["function"]?.jsonObject?.get("name")?.jsonPrimitive?.contentOrNull
                ?: tool.jsonObject["name"]?.jsonPrimitive?.contentOrNull
        }.getOrNull()?.trim()?.takeIf(String::isNotEmpty)
    }.distinct()
}

/** Project ordered Core items without merging tool batches across their results. */
internal fun androidCoreModelMessages(items: List<JsonElement>): List<AgentCoreMessage> {
    val messages = mutableListOf<AgentCoreMessage>()
    items.forEach { element ->
        val item = element as? JsonObject ?: return@forEach
        when (itemString(item, "type")) {
            "message" -> messages += AgentCoreMessage(
                role = itemString(item, "role").orEmpty(),
                content = itemString(item, "content").orEmpty(),
            )
            "tool_call" -> {
                val call = buildJsonObject {
                    put("id", itemString(item, "call_id").orEmpty())
                    put("type", "function")
                    put(
                        "function",
                        buildJsonObject {
                            put("name", itemString(item, "name").orEmpty())
                            put("arguments", (item["arguments"] ?: JsonObject(emptyMap())).toString())
                        },
                    )
                }
                val index = messages.lastIndex
                if (index >= 0 && messages[index].role == "assistant") {
                    messages[index] = messages[index].copy(
                        toolCalls = messages[index].toolCalls + call,
                    )
                } else {
                    messages += AgentCoreMessage(
                        role = "assistant",
                        content = "",
                        toolCalls = listOf(call),
                    )
                }
            }
            "tool_result" -> messages += AgentCoreMessage(
                role = "tool",
                content = (item["output"] ?: JsonNull).toString(),
                name = itemString(item, "name"),
                toolCallId = itemString(item, "call_id"),
            )
            "compaction" -> messages += AgentCoreMessage(
                role = "system",
                content = itemString(item, "summary").orEmpty(),
            )
        }
    }
    return messages
}


private fun itemString(item: JsonObject, key: String): String? =
    (item[key] as? JsonPrimitive)?.contentOrNull
