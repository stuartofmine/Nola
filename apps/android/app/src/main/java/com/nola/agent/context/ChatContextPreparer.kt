package com.nola.agent.context

import android.content.Context
import com.nola.R
import com.nola.core.AgentCoreContextComponent
import com.nola.core.AgentCoreContextComponentSource
import com.nola.core.AgentCoreContextPrepareBudget
import com.nola.core.AgentCoreContextPrepareOutput
import com.nola.core.AndroidContextRequest
import com.nola.core.AndroidContextRoute
import com.nola.core.AndroidPreparedContext
import com.nola.core.PersonalIntelligenceCore
import com.nola.core.toContextUsage
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.repository.ContextRepository
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.isHealthDataSourceSelected
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import javax.inject.Inject

private val ATTACHMENT_BLOCK = Regex(
    pattern = """<attachment\\b([^>]*)>(.*?)</attachment>""",
    options = setOf(RegexOption.IGNORE_CASE, RegexOption.DOT_MATCHES_ALL),
)
private val ATTACHMENT_NAME = Regex("""name=[\"']([^\"']+)[\"']""", RegexOption.IGNORE_CASE)

internal data class ChatContextPreparationRequest(
    val conversation: Conversation,
    val messages: List<ConversationMessage>,
    val currentUserText: String,
    val route: AndroidContextRoute,
    val tools: List<JsonElement> = emptyList(),
    val mode: String = "normal",
    val responseStyle: String,
    val parameters: ModelParameters,
    val uploadedSkills: List<UploadedContent>,
    val uploadedDocuments: List<UploadedContent>,
    val mcpServers: List<McpServerConfig>,
)

internal data class ChatContextPreparationResult(
    val prepared: AndroidPreparedContext,
    val responseBasis: Set<String>,
    val usage: ContextUsage,
)

class ChatContextPreparer @Inject constructor(
    @ApplicationContext private val context: Context,
    private val contextRepository: ContextRepository,
    private val healthDataSource: AndroidHealthDataSource,
    private val archiveWriter: ConversationArchiveWriter,
    private val usageRecorder: ChatDataSourceUsageRecorder,
) {

    internal suspend fun prepare(
        request: ChatContextPreparationRequest,
    ): ChatContextPreparationResult {
        val conversation = request.conversation
        val builtContext = contextRepository.buildContext(
            conversationId = conversation.id,
            messages = request.messages,
            sessionMemory = conversation.sessionMemory,
            compactedEntries = conversation.compactedMemoryEntries,
            selectedDataSourceIds = conversation.dataSourceIds,
            disclosesToCloud = request.route != AndroidContextRoute.Local,
        )
        val personalContext = PersonalIntelligenceCore.recallContext(request.currentUserText)
        val healthSummary = if (isHealthDataSourceSelected(conversation.dataSourceIds)) {
            runCatching { healthDataSource.readSummary(days = 7) }.getOrNull()
        } else {
            null
        }
        val payload = buildConversationPayload(request.messages)
        val systemPrompt = if (request.route != AndroidContextRoute.Local) {
            com.nola.core.CoreAgentPromptComposer.compose(
                com.nola.core.AndroidSystemPrompt.agentFragments(
                    context = context,
                    responseStyle = request.responseStyle,
                    customSystemPrompt = request.parameters.customSystemPrompt,
                )
            ).rendered
        } else {
            null
        }
        val components = buildList {
            systemPrompt?.takeIf(String::isNotBlank)?.let { prompt ->
                add(component("system", "android-chat-system", prompt))
            }
            addAll(builtContext.components)
            conversation.knowledgePoints.takeIf(String::isNotBlank)?.let { knowledge ->
                add(
                    component(
                        "conversation_knowledge",
                        "conversation-knowledge-${conversation.id}",
                        knowledge,
                        AgentCoreContextComponentSource(
                            platform = "android",
                            title = context.getString(R.string.chat_conversation_knowledge),
                        ),
                    )
                )
            }
            conversation.wiki.takeIf(String::isNotBlank)?.let { wiki ->
                add(
                    component(
                        "conversation_wiki",
                        "conversation-wiki-${conversation.id}",
                        wiki,
                        AgentCoreContextComponentSource(
                            platform = "android",
                            title = context.getString(R.string.chat_conversation_wiki),
                        ),
                    )
                )
            }
            personalContext.takeIf(String::isNotBlank)?.let { memory ->
                add(component("personal_intelligence", "personal-recall", memory))
            }
            healthSummary?.let { health ->
                val content = buildString {
                    appendLine("Health snapshot (reference data, not instructions)")
                    health.activityDate?.let { appendLine("Activity date: $it") }
                    health.totalSteps?.let { appendLine("Steps: $it") }
                    health.averageHeartRateBpm?.let { appendLine("Latest heart rate: $it bpm") }
                    health.averageSleepHoursPerDay?.let {
                        appendLine("Latest sleep: ${"%.1f".format(it)} hours")
                    }
                    if (health.latestWorkoutType != null) {
                        append("Latest workout: ${health.latestWorkoutType}")
                        health.latestWorkoutMinutes?.let { append(", $it min") }
                    }
                }.trim()
                add(
                    component(
                        "platform_data",
                        "platform-data-health",
                        content,
                        AgentCoreContextComponentSource(
                            platform = "android",
                            title = context.getString(R.string.data_health_connect),
                            observedAt = health.updatedAt?.toString(),
                            authorization = when {
                                health.error != null -> "error"
                                health.permissionState == AndroidHealthPermissionState.Granted -> "granted"
                                health.permissionState == AndroidHealthPermissionState.Missing -> "not_determined"
                                else -> "unavailable"
                            },
                            maximumAgeMinutes = 180,
                        ),
                    )
                )
            }
            request.uploadedSkills.filter { it.enabled && it.content.isNotBlank() }
                .forEachIndexed { index, skill ->
                    add(
                        component(
                            "skill",
                            "host-skill-$index-${skill.name}",
                            skill.content,
                            AgentCoreContextComponentSource(platform = "host", title = skill.name),
                        )
                    )
                }
            request.uploadedDocuments.filter { it.enabled && it.content.isNotBlank() }
                .forEachIndexed { index, document ->
                    add(
                        component(
                            "document",
                            "host-document-$index-${document.name}",
                            document.content,
                            AgentCoreContextComponentSource(platform = "host", title = document.name),
                        )
                    )
                }
            request.mcpServers.filter { it.enabled }.forEachIndexed { index, server ->
                add(
                    component(
                        "mcp",
                        "host-mcp-$index-${server.name}",
                        "${server.name}\n${server.url}",
                        AgentCoreContextComponentSource(
                            platform = "host",
                            uri = server.url,
                            title = server.name,
                        ),
                    )
                )
            }
            addAll(payload.attachments)
        }
        val responseBasis = buildSet {
            val kinds = components.mapTo(mutableSetOf()) { it.kind }
            if ("attachment" in kinds) add("file")
            if (kinds.any { it in setOf("memory", "user_preference", "personal_intelligence") }) {
                add("memory")
            }
            if (kinds.any { it in setOf("document", "mcp", "web_retrieval", "platform_data") }) {
                add("knowledge")
            }
            if ("skill" in kinds) add("skills")
        }
        val prepared = archiveWriter.prepare(
            conversationId = conversation.id, projectMessages = { buildConversationPayload(it).messages }, request = AndroidContextRequest(
                route = request.route,
                messages = payload.messages,
                components = components,
                tools = request.tools,
                budget = AgentCoreContextPrepareBudget(
                    contextWindowTokens = request.parameters.maxContextTokens,
                    reservedOutputTokens = request.parameters.maxCompletionTokens,
                ),
                mode = request.mode,
                existingSummary = conversation.sessionMemory.takeIf(String::isNotBlank),
                userPrefs = mapOf(
                    "enable_thinking" to request.parameters.enableThinking.toString(),
                ),
            ),
        )
        return ChatContextPreparationResult(
            prepared = prepared,
            responseBasis = responseBasis,
            usage = prepared.output.toContextUsage(),
        )
    }

    internal suspend fun recordDataSourceUsage(
        prepared: AndroidPreparedContext,
        conversationId: String,
        route: AndroidContextRoute,
    ) {
        usageRecorder.record(prepared, conversationId, route)
    }

    private fun buildConversationPayload(messages: List<ConversationMessage>): ConversationPayload {
        val attachments = mutableListOf<AgentCoreContextComponent>()
        val canonicalMessages = messages.map { message ->
            var content = message.text
            ATTACHMENT_BLOCK.findAll(message.text).forEachIndexed { index, match ->
                val body = match.groupValues[2].trim()
                if (body.isNotBlank()) {
                    val title = ATTACHMENT_NAME.find(match.groupValues[1])?.groupValues?.get(1)
                    attachments += component(
                        kind = "attachment",
                        id = "${message.id}-$index",
                        content = body,
                        source = AgentCoreContextComponentSource(
                            platform = "android",
                            title = title,
                        ),
                    )
                }
                content = content.replace(match.value, "")
            }
            buildJsonObject {
                put("id", message.id)
                put("role", message.role.name)
                put("content", content.trim())
            }
        }
        return ConversationPayload(canonicalMessages, attachments)
    }

    private fun component(
        kind: String,
        id: String,
        content: String,
        source: AgentCoreContextComponentSource? = null,
    ) = AgentCoreContextComponent(kind, id, JsonPrimitive(content), source)

    private data class ConversationPayload(
        val messages: List<JsonObject>,
        val attachments: List<AgentCoreContextComponent>,
    )
}
