package com.nola.core

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidQueuedUiInputTest {
    private val submission = Json.parseToJsonElement("""{
      "type":"user_input","submission_id":"command","items":[{
      "type":"ui_action","id":"ui:command:user","action":{
      "surface":{"identity":{"owner":{"authority_id":"host","conversation_id":"conversation",
      "participant_id":"agent","context_id":"context","originating_run_id":"run","message_id":"original",
      "part_id":"tool"},"surface_id":"form","generation":1},"revision":2},
      "envelope":{"version":"v0.9.1","action":{"name":"submit","context":{"name":"Android验收"}}},
      "summary":"Submitted name"}}]}""").jsonObject

    @Test fun typedQueueUsesHumanSummaryAndRetainsCanonicalItemIdentity() {
        assertEquals("ui:command:user" to "Submitted name", queuedUiInputPresentation(submission, "conversation", "command"))
        assertTrue(submission.toString().contains("Android验收"))
    }
    @Test fun foreignConversationOrCommandCannotBeClaimedAsThisAction() {
        assertTrue(runCatching { queuedUiInputPresentation(submission, "foreign", "command") }.isFailure)
        assertTrue(runCatching { queuedUiInputPresentation(submission, "conversation", "other") }.isFailure)
    }
}
