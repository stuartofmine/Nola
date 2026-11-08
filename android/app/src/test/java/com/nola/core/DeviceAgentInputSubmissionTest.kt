package com.nola.core

import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class DeviceAgentInputSubmissionTest {
    @Test fun cancellationHasItsOwnRequiredOperationIdentity() {
        val first = deviceAgentCancellationSubmission("user_requested")
        val second = deviceAgentCancellationSubmission("user_requested")
        assertEquals("interrupt", first.getValue("type").jsonPrimitive.content)
        assertEquals("user_requested", first.getValue("reason").jsonPrimitive.content)
        java.util.UUID.fromString(first.getValue("cancellation_id").jsonPrimitive.content)
        assertFalse(first["cancellation_id"] == second["cancellation_id"])
    }

    @Test
    fun interactiveInputUsesUserInputContractByDefault() {
        val submission = encodeDeviceAgentInputSubmission(
            input = "Hello",
            submissionId = "submission-1",
            messageId = "message-1",
            submittedAtEpochMs = 1_785_600_000_123,
        )

        assertEquals("user_input", submission.getValue("type").jsonPrimitive.content)
        assertEquals("current_turn", submission.getValue("delivery").jsonPrimitive.content)
        assertFalse(submission.containsKey("origin"))
        assertEquals(
            "1785600000123",
            submission.getValue("submitted_at_epoch_ms").jsonPrimitive.content,
        )
        assertEquals(
            "Hello",
            submission.getValue("items").jsonArray.single().jsonObject
                .getValue("content").jsonPrimitive.content,
        )
    }

    @Test
    fun productionInputConstructsQueuedAdmissionWithCompleteContextEvenWhenEmpty() {
        val submission = encodeDeviceAgentInputSubmission("human summary", submissionId = "queued", messageId = "ui:queued:user")
        val descriptor = AndroidCanonicalTurnDescriptor("conversation", "", "queued", "ui:queued:user", "human summary", null,
            expectedConfigurationRevision = 7,
            queuedUserAdmission = AndroidQueuedUserAdmission("queued", "lease", 1, "ui:queued:user"))
        val command = encodeQueuedUserTurnAdmission(descriptor, JsonObject(emptyMap()), submission, 123)
        assertEquals("begin_queued_user_turn", command.getValue("type").jsonPrimitive.content)
        val input = command.getValue("input").jsonObject
        assertEquals("android", input.getValue("turn_context_platform").jsonPrimitive.content)
        assertEquals(0, input.getValue("turn_context_items").jsonArray.size)
        assertEquals(0, input.getValue("turn_context_components").jsonArray.size)
        assertEquals("queued", input.getValue("command_id").jsonPrimitive.content)
        assertFalse(input.containsKey("submission"))
        assertFalse(input.containsKey("user_text"))
    }

    @Test
    fun scheduledActionUsesAutomatedInputContract() {
        val reference = AgentCoreContextComponent(
            kind = "reference",
            id = "source-1",
            content = JsonPrimitive("untrusted source material"),
            source = AgentCoreContextComponentSource(
                platform = "android",
                uri = "https://example.com/feed.json",
                authorization = "authorized",
            ),
        )
        val submission = encodeDeviceAgentInputSubmission(
            input = "Run the scheduled report",
            inputOrigin = DeviceAgentInputOrigin.SCHEDULED_ACTION,
            turnContextComponents = listOf(reference),
            submissionId = "submission-2",
            messageId = "message-2",
            submittedAtEpochMs = 1_785_600_000_456,
        )

        assertEquals("automated_input", submission.getValue("type").jsonPrimitive.content)
        assertEquals("scheduled_action", submission.getValue("origin").jsonPrimitive.content)
        assertFalse(submission.containsKey("delivery"))
        assertEquals("submission-2", submission.getValue("submission_id").jsonPrimitive.content)
        assertEquals(
            "1785600000456",
            submission.getValue("submitted_at_epoch_ms").jsonPrimitive.content,
        )
        assertEquals("android", submission.getValue("turn_context_platform").jsonPrimitive.content)
        assertEquals(
            "reference",
            submission.getValue("turn_context_components").jsonArray.single().jsonObject
                .getValue("kind").jsonPrimitive.content,
        )
    }

    @Test
    fun foregroundAndColdWorkerUseAnExtractionEnabledMemoryPolicy() {
        val policy = automaticMemoryPolicyConfig()

        assertEquals(
            "true",
            policy.getValue("long_term_enabled").jsonPrimitive.content,
        )
        assertEquals(
            "true",
            policy.getValue("automatic_extraction_enabled").jsonPrimitive.content,
        )
        assertEquals(
            0,
            policy.getValue("allowed_sensitivities").jsonArray.size,
        )
    }
}
