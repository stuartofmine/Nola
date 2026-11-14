// SPDX-License-Identifier: Apache-2.0
package com.nola.core

import com.nola.data.local.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidEditedResendContractTest {
    private val profile = AndroidModelCapabilityProfile("android-text-adapter-v1:Cloud:original", 7,
        nativeToolCalls = true, reasoning = true, visionInput = false,
        nativeEventStream = true, contextWindowTokens = 64000, maxOutputTokens = 4000)

    @Test fun originalCapabilityRoundTripsWithoutCurrentRegistryOrHostIdentity() {
        val frozen = profile.wire()
        assertEquals(frozen, frozenAndroidModelCapability(frozen).wire())
        assertEquals(7L, frozenAndroidModelCapability(frozen).revision)
        for (changed in listOf(
            JsonObject(frozen + ("unknown_capability" to JsonPrimitive(true))),
            JsonObject(frozen - "revision"),
            JsonObject(frozen + ("context_window_tokens" to JsonPrimitive(0))),
            JsonObject(frozen + ("direct_tool_capability_ids" to JsonArray(listOf(JsonPrimitive("old.file.read"))))),
            JsonObject(frozen + ("stream_reasoning" to JsonPrimitive(false))),
        )) assertTrue(runCatching { frozenAndroidModelCapability(changed) }.isFailure)
    }

    private fun prepared(): AndroidPreparedEditedResend {
        val operation = "3fcd5299-0cad-46e1-b25d-1472a6500123"
        val input = AndroidPrepareEditedResend(operation, "source", "source:user", 19, 3,
            " edited input \n", "6129ed80-46b2-4892-8613-0fd68112b543", "Edit", 123)
        val model = AndroidFrozenResendModel(8, buildJsonObject { put("kind", "http") },
            JsonArray(emptyList()), buildJsonObject {}, buildJsonObject {}, profile.wire())
        return AndroidPreparedEditedResend(input, AndroidMessageSendWitness("source", "source:user", "source-run",
            "source:assistant", "body", "invocation", model), "$operation:user", "$operation:assistant", operation, null)
    }

    @Test fun preparationIsNotAdmissionAndStableIdsCannotBeRebound() {
        val pending = prepared()
        pending.requireIdentity()
        assertNull(pending.admittedRunId)
        assertEquals(" edited input \n", pending.input.editedText)
        pending.copy(admittedRunId = pending.runId).requireIdentity()
        for (changed in listOf(
            pending.copy(userMessageId = "source:user"),
            pending.copy(assistantMessageId = "source:assistant"),
            pending.copy(admittedRunId = "other-run"),
            pending.copy(source = pending.source.copy(messageId = "another:user")),
            pending.copy(input = pending.input.copy(targetConversationId = "source")),
        )) assertTrue(runCatching { changed.requireIdentity() }.isFailure)
    }

    @Test fun customInstructionsProduceAPlainTypedDefinitionWithoutOldSystemParsing() {
        val definition = AndroidSystemPrompt.customInstructionComponent("  Answer briefly.  ")!!
        assertEquals("prompt_template", definition.kind)
        assertEquals("android:user-custom-instructions", definition.id)
        assertEquals(JsonPrimitive("Answer briefly."), definition.content)
        assertNull(definition.source?.authorization)
        assertNull(definition.source?.uri)
        assertNull(AndroidSystemPrompt.customInstructionComponent("  "))
    }

    @Test fun inPlaceResendKeepsTheOwnerAndUsesAnAdditiveContract() {
        val original = prepared().input
        val input = original.inPlace()
        assertEquals(original.sourceConversationId, input.conversationId)
        assertEquals(original.sourceMessageId, input.messageId)
        assertEquals(original.operationId, input.operationId)
        assertEquals(original.expectedSourceRevision, input.expectedRevision)
        assertEquals(original.editedText, input.editedText)
        val wire = Json.encodeToJsonElement(AndroidPrepareInPlaceResend.serializer(), input).jsonObject
        assertFalse("target_conversation_id" in wire)
        assertTrue(ApplicationCommandEffect.isObservation(buildJsonObject { put("type", "load_prepared_in_place_resend") }))
        assertFalse(ApplicationCommandEffect.isObservation(buildJsonObject { put("type", "prepare_in_place_resend") }))
    }
    @Test fun publicQueryBoundaryCannotAttestHostPolicy() {
        assertTrue(ApplicationCommandEffect.isObservation(buildJsonObject {
            put("type", "load_prepared_edited_resend"); put("conversation_id", "target")
        }))
        assertFalse(ApplicationCommandEffect.isObservation(buildJsonObject { put("type", "attest_edited_resend_host_policy") }))
        assertFalse(ApplicationCommandEffect.isObservation(buildJsonObject { put("type", "prepare_edited_resend") }))
    }
}
