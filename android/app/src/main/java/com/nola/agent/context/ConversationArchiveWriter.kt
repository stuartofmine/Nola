package com.nola.agent.context

import com.nola.core.*
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.repository.ConversationRepository
import com.nola.domain.model.ModelInput
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.serialization.json.*
import javax.inject.Inject

/** Executes Application's maintenance transaction; never independently patches
 * session memory and lineage. Source changes reject the entire commit. */
class ConversationArchiveWriter @Inject constructor(
    private val conversationRepository: ConversationRepository,
    private val application: AndroidApplicationRuntime,
    private val summaryService: ChatContextSummaryService,
) {
    suspend fun prepare(conversationId: String, request: AndroidContextRequest,
        projectMessages: (List<com.nola.domain.model.ConversationMessage>) -> List<JsonObject>): AndroidPreparedContext {
        val source = application.prepareContextCompaction(conversationId)
        val conversation = requireNotNull(conversationRepository.getConversation(conversationId))
        val canonical = projectMessages(conversationRepository.getActiveMessages(conversationId))
        val canonicalIDs = canonical.mapNotNull { it["id"]?.jsonPrimitive?.content }.toSet()
        val pending = request.messages.filter { it["id"]?.jsonPrimitive?.content !in canonicalIDs }.map {
            JsonObject(it + ("ephemeral" to JsonPrimitive(true)))
        }
        val original = AgentCoreContextPrepareInput(mode = request.mode, components = request.components,
            messages = canonical + pending, tools = request.tools, budget = request.budget, existingSummary = conversation.sessionMemory)
        val plan = application.execute(buildJsonObject {
            put("type", "context"); put("command", buildJsonObject {
                put("type", "prepare_maintenance"); put("source", source)
                put("input", Json.encodeToJsonElement(original))
            })
        }).jsonObject
        var output = Json.decodeFromJsonElement<AgentCoreContextPrepareOutput>(plan.getValue("output"))
        if (output.action == "summary_required") {
            val archive = requireNotNull(output.archiveRequest)
            val instructions = plan.getValue("instructions").jsonPrimitive.content
            val summary = summaryService.summarize(conversationId, archive, instructions,
                requireNotNull(request.budget.contextWindowTokens) { "Compaction context window is missing" },
                request.budget.reservedOutputTokens)
            currentCoroutineContext().ensureActive()
            val validated = application.prepareContextCompaction(conversationId)
            val conversation = requireNotNull(conversationRepository.getConversation(conversationId))
            val messages = conversationRepository.getActiveMessages(conversationId)
            // Recheck the complete current canonical text, conservatively including
            // attachment text. Only the first prepared input determines archive IDs.
            val current = original.copy(existingSummary = conversation.sessionMemory,
                messages = messages.map { message -> buildJsonObject {
                    put("id", message.id); put("role", message.role.name); put("content", message.text)
                } } + pending)
            output = Json.decodeFromJsonElement(application.execute(buildJsonObject {
                put("type", "context"); put("command", buildJsonObject {
                    put("type", "commit_maintenance"); put("input", buildJsonObject {
                        put("source", plan.getValue("source")); put("validated_source", validated)
                        put("original_input", plan.getValue("input")); put("current_input", Json.encodeToJsonElement(current))
                        put("summary", summary); put("committed_at_ms", System.currentTimeMillis())
                    })
                })
            }))
        }
        check(output.action == "ready") { "Application did not prepare a ready context" }
        return AndroidPreparedContext(request.route, ModelInput(userPrefs = request.userPrefs,
            canonicalMessages = output.messages, tools = output.tools, contextPayloadDigest = output.payloadDigest), output)
    }
}
