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
import com.nola.data.health.AndroidHealthSummary
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
    pattern = """<attachment\b([^>]*)>(.*?)</attachment>""",
    options = setOf(RegexOption.IGNORE_CASE, RegexOption.DOT_MATCHES_ALL),
)
private val ATTACHMENT_NAME = Regex("""name=["']([^"']+)["']""", RegexOption.IGNORE_CASE)

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
        val payload = extractConversationPayload(request.messages)
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
        val components = assembleChatContextComponents(
            systemPrompt = systemPrompt,
            builtComponents = builtContext.components,
            conversation = conversation,
            knowledgeTitle = context.getString(R.string.chat_conversation_knowledge),
            wikiTitle = context.getString(R.string.chat_conversation_wiki),
            personalContext = personalContext,
            healthSummary = healthSummary,
            healthTitle = context.getString(R.string.data_health_connect),
            uploadedSkills = request.uploadedSkills,
            uploadedDocuments = request.uploadedDocuments,
            attachments = payload.attachments,
        )
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
            conversationId = conversation.id, projectMessages = { extractConversationPayload(it).messages }, request = AndroidContextRequest(
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
}

internal data class ConversationPayload(
    val messages: List<JsonObject>,
    val attachments: List<AgentCoreContextComponent>,
)

internal fun extractConversationPayload(messages: List<ConversationMessage>): ConversationPayload {
    val attachments = mutableListOf<AgentCoreContextComponent>()
    val canonicalMessages = messages.map { message ->
        var content = message.text
        ATTACHMENT_BLOCK.findAll(message.text).forEachIndexed { index, match ->
            val body = match.groupValues[2].trim()
            if (body.isNotBlank()) {
                val title = ATTACHMENT_NAME.find(match.groupValues[1])?.groupValues?.get(1)
                attachments += contextComponent(
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

internal fun assembleChatContextComponents(
    systemPrompt: String?,
    builtComponents: List<AgentCoreContextComponent>,
    conversation: Conversation,
    knowledgeTitle: String,
    wikiTitle: String,
    personalContext: String?,
    healthSummary: AndroidHealthSummary?,
    healthTitle: String,
    uploadedSkills: List<UploadedContent>,
    uploadedDocuments: List<UploadedContent>,
    attachments: List<AgentCoreContextComponent>,
): List<AgentCoreContextComponent> = buildList {
    systemPrompt?.takeIf(String::isNotBlank)?.let { prompt ->
        add(contextComponent("system", "android-chat-system", prompt))
    }
    addAll(builtComponents)
    conversation.knowledgePoints.takeIf(String::isNotBlank)?.let { knowledge ->
        add(
            contextComponent(
                "document",
                "conversation-knowledge-${conversation.id}",
                knowledge,
                AgentCoreContextComponentSource(
                    platform = "android",
                    title = knowledgeTitle,
                ),
            )
        )
    }
    conversation.wiki.takeIf(String::isNotBlank)?.let { wiki ->
        add(
            contextComponent(
                "document",
                "conversation-wiki-${conversation.id}",
                wiki,
                AgentCoreContextComponentSource(
                    platform = "android",
                    title = wikiTitle,
                ),
            )
        )
    }
    personalContext?.takeIf(String::isNotBlank)?.let { memory ->
        add(contextComponent("personal_intelligence", "personal-recall", memory))
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
            contextComponent(
                "platform_data",
                "platform-data-health",
                content,
                AgentCoreContextComponentSource(
                    platform = "android",
                    title = healthTitle,
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
    val existingSkillComponentIds = builtComponents
        .filter { it.kind == "skill" }
        .mapTo(mutableSetOf()) { it.id }
    val existingSkillTitles = builtComponents
        .filter { it.kind == "skill" }
        .mapNotNullTo(mutableSetOf()) { it.source?.title }
    val existingDocumentTitles = builtComponents
        .filter { it.kind == "document" }
        .mapNotNullTo(mutableSetOf()) { it.source?.title }
    uploadedSkills
        .filter { it.enabled && it.name.isNotBlank() }
        .filter { skill ->
            val skillId = skill.resourceId?.trim()?.takeIf(String::isNotEmpty) ?: skill.name
            existingSkillComponentIds.add("skill:$skillId") && existingSkillTitles.add(skill.name)
        }
        .forEach { skill ->
            val skillId = skill.resourceId?.trim()?.takeIf(String::isNotEmpty) ?: skill.name
            val summary = skill.description.trim()
                .ifEmpty {
                    skill.content.lineSequence()
                        .map(String::trim)
                        .firstOrNull { it.isNotEmpty() && !it.startsWith("#") }
                        ?.take(240)
                        .orEmpty()
                }
                .ifEmpty { skill.name }
            val metadata = buildJsonObject {
                put("schema_version", 1)
                put("type", "available_skill")
                put("id", skillId)
                put("name", skill.name)
                put("summary", summary)
                put("usage_contract", "Metadata summary only; call skill_invoke before claiming Skill instructions.")
                put(
                    "invocation",
                    buildJsonObject {
                        put("tool", "skill_invoke")
                        put("arguments", buildJsonObject { put("skill", skill.name) })
                    },
                )
            }.toString()
            add(
                contextComponent(
                    "skill",
                    "skill:$skillId",
                    metadata,
                    AgentCoreContextComponentSource(platform = "android", title = skill.name),
                )
            )
        }
    uploadedDocuments
        .filter { it.enabled && it.content.isNotBlank() }
        .filter { document -> existingDocumentTitles.add(document.name) }
        .forEachIndexed { index, document ->
            add(
                contextComponent(
                    "document",
                    "host-document-$index-${document.name}",
                    document.content,
                    AgentCoreContextComponentSource(platform = "host", title = document.name),
                )
            )
        }
    addAll(attachments)
}

internal fun nonSystemTurnContextComponents(
    components: List<AgentCoreContextComponent>,
): List<AgentCoreContextComponent> = components.filter { it.kind != "system" }

private fun contextComponent(
    kind: String,
    id: String,
    content: String,
    source: AgentCoreContextComponentSource? = null,
) = AgentCoreContextComponent(kind, id, JsonPrimitive(content), source)
