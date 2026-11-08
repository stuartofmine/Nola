// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import android.content.Context
import com.nola.core.AgentCoreResourcePrincipal
import com.nola.core.AgentCoreResourcePrincipalKind
import com.nola.core.AndroidSystemPrompt
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.data.repository.applyModelPresentation
import com.nola.data.repository.readConversationParameters
import com.nola.data.repository.toConfiguredModelInfo
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import java.security.MessageDigest
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.json.*

/** Authenticated local native Host factory. Renderer JSON cannot supply an actor,
 * builtin catalog, definition body, authorization clock or private operation. */
internal class AndroidExpertHostFactory(context: Context, private val runtime: AndroidApplicationRuntime) {
    private val profiles = MemoryProfileDataStore(context, SecureTokenStore(context), runtime)

    private fun authority(): JsonObject = buildJsonObject {
        val id = runBlocking { profiles.getOrCreateResourcePrincipalId() }
        put("actor", Json.encodeToJsonElement(AgentCoreResourcePrincipal.serializer(),
            AgentCoreResourcePrincipal(AgentCoreResourcePrincipalKind.LocalUser, id)))
        // Prompt resources are local to this authenticated Store. A tool Host is
        // not their resource Host and must never be guessed from a model URL.
        put("request_host_id", JsonNull)
    }

    fun pendingAdmission(conversation: String): AndroidPendingExpertAdmission? {
        require(runtime.conversationSourceOwner(conversation) == ConversationSourceOwner.Local) { "The expert source Store is unavailable" }
        return runtime.pendingExpertHostOperations(authority()).filter { it.operation["kind"] == JsonPrimitive("admit") }
            .map { AndroidPendingExpertAdmission(it.requestId, Json.decodeFromJsonElement<AndroidAdmitExpertMessage>(it.operation.getValue("input"))) }
            .filter { it.input.conversationId == conversation }
            .also { require(it.size <= 1) { "The pending expert source is not unique" } }.singleOrNull()
    }

    fun pendingEdit(conversation: String, message: String): AndroidPendingExpertEdit? {
        require(runtime.conversationSourceOwner(conversation) == ConversationSourceOwner.Local) { "The expert source Store is unavailable" }
        return runtime.pendingExpertHostOperations(authority()).filter { it.operation["kind"] == JsonPrimitive("prepare_edited_resend") }
            .map { AndroidPendingExpertEdit(it.requestId, Json.decodeFromJsonElement<AndroidPrepareEditedResend>(it.operation.getValue("input"))) }
            .filter { it.input.sourceConversationId == conversation && it.input.sourceMessageId == message }
            .also { require(it.size <= 1) { "The pending expert edit is not unique" } }.singleOrNull()
    }

    fun checkAdmission(pending: AndroidPendingExpertAdmission): AndroidExpertMessageManifest {
        val original = runtime.pendingExpertHostOperations(authority()).singleOrNull { it.requestId == pending.requestId }
            ?: return checkNotNull(manifest(pending.input.conversationId, pending.input.userMessageId)).also {
                require(it.input == pending.input) { "The original expert request changed" }
            }
        require(original.operation["kind"] == JsonPrimitive("admit") &&
            original.operation["input"] == Json.encodeToJsonElement(pending.input)) { "The original native Host request changed" }
        return Json.decodeFromJsonElement(runtime.executeExpertHostMutation(original.operation))
    }

    fun manifest(conversation: String, message: String): AndroidExpertMessageManifest? =
        manifest(conversation, message, authority())

    private fun manifest(conversation: String, message: String, actor: JsonObject): AndroidExpertMessageManifest? =
        decodeOptional(runtime.executeExpertHostQuery(buildJsonObject {
            put("kind", "load_manifest"); put("conversation_id", conversation); put("message_id", message)
            put("authority", actor)
        }))

    fun admit(input: AndroidAdmitExpertMessage): AndroidExpertMessageManifest {
        val actor = authority()
        val encoded = Json.encodeToJsonElement(input)
        // An ambiguous call retains its original full native context. Never
        // rebuild that context from today's changed templates/model settings.
        runtime.pendingExpertHostMutation("admit", encoded, actor)?.let {
            return decodeAdmission(input, it)
        }
        // Recover a committed original intent before touching today's templates
        // or user instructions: deletion/revocation must not erase its receipt.
        manifest(input.conversationId, input.userMessageId, actor)?.let { prior ->
            require(prior.input == input) { "The original expert intent differs from this request" }
            return prior
        }
        val preview = Json.decodeFromJsonElement<AndroidExpertCapturePreview>(runtime.executeExpertHostQuery(buildJsonObject {
            put("kind", "preview_capture"); put("input", encoded); put("authority", actor)
        }))
        require(preview.participants.map { it.participant } == input.selectedParticipants) {
            "Expert capture preview changed the complete participant set"
        }
        val selections = preview.participants.map { entry ->
            val settings = entry.defaults
            val template = settings["selectedPromptTemplateID"]?.takeUnless { it == JsonNull }?.jsonPrimitive
            require(template == null || (template.isString && template.content.isNotBlank())) { "Invalid expert template selection" }
            val definitions = userDefinitions(settings, entry.modelDeclaration, entry.clientModelPresentation)
            buildJsonObject {
                put("context_conversation_id", entry.participant.contextConversationId)
                put("context_configuration_revision", entry.contextConfigurationRevision)
                put("source", if (template != null) "explicit" else "no_template")
                put("template_id", template ?: JsonNull)
                put("model_profile", buildJsonObject {
                    put("profile_id", entry.modelDeclaration.getValue("profile_id"))
                    put("admitted_revision", entry.modelDeclaration.getValue("profile_revision"))
                })
                put("host_user_instructions", Json.encodeToJsonElement(definitions))
            }
        }
        val capture = buildJsonObject {
            put("actor", actor.getValue("actor")); put("request_host_id", actor.getValue("request_host_id"))
            put("selections", JsonArray(selections)); put("builtins", JsonArray(emptyList()))
        }
        return decodeAdmission(input, buildJsonObject {
            put("kind", "admit"); put("input", encoded); put("context", capture)
        })
    }

    private fun decodeAdmission(input: AndroidAdmitExpertMessage, operation: JsonObject): AndroidExpertMessageManifest = try {
        Json.decodeFromJsonElement(runtime.executeExpertHostMutation(operation))
    } catch (pending: AndroidApplicationRuntime.ExpertHostPending) {
        throw AndroidExpertAdmissionOutcomePending(AndroidPendingExpertAdmission(pending.requestId, input), pending.reopenRequired)
    }

    fun prepared(target: String): AndroidPreparedExpertResend? = decodeOptional(runtime.executeExpertHostQuery(buildJsonObject {
        put("kind", "load_prepared_resend"); put("conversation_id", target); put("authority", authority())
    }))

    fun prepare(input: AndroidPrepareEditedResend): AndroidPreparedExpertResend {
        val actor = authority()
        val encoded = Json.encodeToJsonElement(input)
        val operation = runtime.pendingExpertHostMutation("prepare_edited_resend", encoded, actor) ?: buildJsonObject {
            put("kind", "prepare_edited_resend"); put("input", encoded); put("authority", actor)
        }
        return Json.decodeFromJsonElement(runtime.executeExpertHostMutation(operation))
    }

    fun childInstructions(conversation: String, expectedRun: String): List<AndroidExpertPromptDefinition>? =
        decodeOptional(runtime.executeExpertHostQuery(buildJsonObject {
            put("kind", "load_child_instructions"); put("conversation_id", conversation)
            put("expected_run_id", expectedRun); put("authority", authority())
        }))

    fun readableTemplates(): ReadableExpertTemplateCatalog = Json.decodeFromJsonElement(
        runtime.executeExpertHostQuery(buildJsonObject { put("kind", "readable_templates"); put("authority", authority()) }))

    /** The shared admission preview is the only model-selection authority. The
     * existing native parameter reader remains unchanged in this source batch. */
    private fun userDefinitions(settings: JsonObject, declaration: JsonObject, presentation: JsonElement): List<AndroidExpertPromptDefinition> {
        val id = declaration.getValue("profile_id").jsonPrimitive.content
        val revision = declaration.getValue("profile_revision").jsonPrimitive.long
        val profile = checkNotNull(runtime.modelProfile(id)) { "The exact previewed expert model is unavailable" }
        require(profile.id == id && profile.revision == revision && profile.enabled && profile.deletedAtMs == null) {
            "The previewed expert model configuration changed"
        }
        require(profile.provider == declaration.getValue("provider").jsonPrimitive.content &&
            profile.modelName == declaration.getValue("model_name").jsonPrimitive.content &&
            profile.runtime == declaration.getValue("runtime").jsonPrimitive.content &&
            Json.parseToJsonElement(profile.parametersJson) == declaration.getValue("parameters")) {
            "The exact previewed expert model declaration changed"
        }
        val model = if (profile.runtime == "remote") profile.toConfiguredModelInfo() else
            ModelInfo(profile.id, profile.displayName, ModelInfo.ModelSource.Local, profileId = profile.id, profileRevision = profile.revision)
        val parameters = readConversationParameters(settings.toString(), model)
            ?: Json.decodeFromJsonElement(ModelParameters.serializer(), declaration.getValue("parameters"))
        val effectivePresentation = applyModelPresentation(parameters, presentation.takeUnless { it == JsonNull }?.jsonObject)
        val component = AndroidSystemPrompt.customInstructionComponent(effectivePresentation.customSystemPrompt) ?: return emptyList()
        val body = component.content.jsonPrimitive.content
        val digest = MessageDigest.getInstance("SHA-256").digest(body.encodeToByteArray()).joinToString("") { "%02x".format(it) }
        return listOf(AndroidExpertPromptDefinition(component.id, body, digest, null, JsonPrimitive("android")))
    }

    private inline fun <reified T> decodeOptional(value: JsonElement): T? =
        if (value == JsonNull) null else Json.decodeFromJsonElement<T>(value)
}

@Serializable
internal data class AndroidExpertCapturePreview(val participants: List<AndroidExpertCapturePreviewEntry>)
@Serializable
internal data class AndroidExpertCapturePreviewEntry(
    val participant: AndroidExpertParticipantReference,
    @SerialName("context_configuration_revision") val contextConfigurationRevision: Long,
    val defaults: JsonObject,
    @SerialName("model_declaration") val modelDeclaration: JsonObject,
    @SerialName("client_model_presentation") val clientModelPresentation: JsonElement,
    @SerialName("effective_parameters") val effectiveParameters: JsonObject,
)

data class AndroidPendingExpertAdmission(val requestId: String, val input: AndroidAdmitExpertMessage)
internal data class AndroidPendingExpertEdit(val requestId: String, val input: AndroidPrepareEditedResend)

internal class AndroidExpertAdmissionOutcomePending(val pending: AndroidPendingExpertAdmission, reopenRequired: Boolean) :
    AndroidApplicationRuntime.ExpertHostPending(pending.requestId, reopenRequired)
