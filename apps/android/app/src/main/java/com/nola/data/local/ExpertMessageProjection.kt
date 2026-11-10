// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.json.*
@Serializable data class AndroidExpertParticipantReference(
    @SerialName("participant_id") val participantId:String,
    @SerialName("agent_id") val agentId:String,
    @SerialName("agent_revision") val agentRevision:Long,
    @SerialName("context_conversation_id") val contextConversationId:String)
@Serializable data class AndroidAdmitExpertMessage(
    @SerialName("conversation_id") val conversationId:String,
    @SerialName("expected_revision") val expectedRevision:Long,
    @SerialName("room_id") val roomId:String,
    @SerialName("expected_room_revision") val expectedRoomRevision:Long,
    @SerialName("submission_id") val submissionId:String,
    @SerialName("user_message_id") val userMessageId:String,
    val text:String,
    @SerialName("selected_participants") val selectedParticipants:List<AndroidExpertParticipantReference>,
    @SerialName("created_at_ms") val createdAtMs:Long)
@Serializable data class AndroidExpertPromptDefinition(
    @SerialName("component_id") val componentId:String,val content:String,
    @SerialName("content_sha256") val contentSha256:String,
    @SerialName("definition_revision") val definitionRevision:Long?,val platform:JsonElement)
@Serializable data class AndroidExpertInstructionWitness(
    @SerialName("checkpoint_sha256") val checkpointSha256:String,
    @SerialName("component_payload_digest") val componentPayloadDigest:String,
    @SerialName("prompt_definitions") val promptDefinitions:List<AndroidExpertPromptDefinition>)
@Serializable data class AndroidExpertMessageEntry(
    val participant:AndroidExpertParticipantReference,
    @SerialName("display_name") val displayName:String,val description:String,val defaults:JsonObject,
    @SerialName("model_declaration") val modelDeclaration:JsonObject,
    @SerialName("context_configuration_revision") val contextConfigurationRevision:Long,
    @SerialName("child_submission_id") val childSubmissionId:String,
    @SerialName("child_user_message_id") val childUserMessageId:String,
    @SerialName("child_assistant_message_id") val childAssistantMessageId:String,
    @SerialName("expected_prompt_definition") val expectedPromptDefinition:AndroidExpertPromptDefinition?,
    @SerialName("model_witness") val modelWitness:AndroidMessageSendWitness?,
    @SerialName("instruction_witness") val instructionWitness:AndroidExpertInstructionWitness?)
@Serializable data class AndroidExpertMessageManifest(val version:Int,val input:AndroidAdmitExpertMessage,val entries:List<AndroidExpertMessageEntry>)
@Serializable data class AndroidPreparedExpertResendEntry(val original:AndroidExpertMessageEntry,
    val participant:AndroidExpertParticipantReference,val prepared:AndroidPreparedEditedResend)
@Serializable data class AndroidPreparedExpertResend(val input:AndroidPrepareEditedResend,
    @SerialName("source_submission_id") val sourceSubmissionId:String,
    @SerialName("room_id") val roomId:String,
    @SerialName("parent_message_id") val parentMessageId:String,
    val entries:List<AndroidPreparedExpertResendEntry>)
internal fun AndroidApplicationRuntime.expertMessageSource(conversation: String, message: String): AndroidExpertMessageManifest? =
    expertHostFactory.manifest(conversation, message)
internal fun AndroidApplicationRuntime.admitExpertMessage(input: AndroidAdmitExpertMessage): AndroidExpertMessageManifest =
    expertHostFactory.admit(input)
internal fun AndroidApplicationRuntime.loadPreparedExpertResend(target: String): AndroidPreparedExpertResend? =
    expertHostFactory.prepared(target)
internal fun AndroidApplicationRuntime.prepareExpertResend(input: AndroidPrepareEditedResend): AndroidPreparedExpertResend =
    expertHostFactory.prepare(input)
internal fun AndroidApplicationRuntime.expertChildInstructions(target: String, expectedRun: String): List<AndroidExpertPromptDefinition>? =
    expertHostFactory.childInstructions(target, expectedRun)
