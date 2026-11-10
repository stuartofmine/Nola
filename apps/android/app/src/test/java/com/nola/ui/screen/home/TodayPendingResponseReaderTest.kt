// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import com.nola.data.local.ApplicationCommandEffect
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Wire/query-contract unit checks. Native UI and Core ownership require separate integration evidence. */
class TodayPendingResponseReaderTest {
    private fun snapshot(): JsonObject = Json.parseToJsonElement("""
        {"needs_attention":false,"unrepresented_expert_contexts":[],"conversations":[
          {"conversation_id":"parent","title":"Original conversation","revision":12,"updated_at_ms":200,
           "needs_response":true,"active_run":null,"permission_projection":null,"open_waits":[],"owned_task_waits":[],
           "owned_task_permissions":[{"task_id":"task","parent_conversation_id":"parent","parent_run_id":"sealed-parent-run",
             "child_conversation_id":"child","child_run_id":"child-run","admission_attempt_id":"attempt",
             "permission_projection":{"conversation_id":"child","run_id":"child-run","permissions":{"pending":[{"request_id":"exact-request"}]}}}],"expert_contexts":[]},
          {"conversation_id":"recent","title":"Recent, not a completed result","revision":9,"updated_at_ms":100,
           "needs_response":false,"active_run":null,"permission_projection":null,"open_waits":[],"owned_task_waits":[],"owned_task_permissions":[],"expert_contexts":[]}
         ],"recent_conversations":[
          {"conversation_id":"global-recent","title":"Global recent outside waiting page","revision":4,"updated_at_ms":999,
           "needs_response":false,"active_run":null,"permission_projection":null,"open_waits":[],"owned_task_waits":[],"owned_task_permissions":[],"expert_contexts":[]}
         ],"has_more_conversations":true,"next_conversation_offset":50,
         "queued_instances":[{"occurrence_id":"occurrence","automation_id":"automation","definition_revision":3,"title":null,"scheduled_for_ms":400}],
         "has_more_queued_instances":true}
    """).jsonObject

    @Test fun oneReadUsesOnlyTheReadOnlyPinnedCoreQueryWithExactBounds() {
        val calls = mutableListOf<JsonObject>()
        val reader = TodayPendingResponseReader { command ->
            assertTrue(ApplicationCommandEffect.isObservation(command), "Production query whitelist must admit Today")
            calls += command
            snapshot()
        }
        val facts = reader.read()
        assertEquals(1, calls.size)
        assertEquals(Json.parseToJsonElement("""{"type":"load_today_facts","input":{"conversation_offset":0,"conversation_limit":50,"queued_limit":10}}"""), calls.single())
        assertEquals("parent", facts.conversations.first().conversationId)
        assertTrue(facts.conversations.first().needsResponse)
        assertNull(facts.conversations.first().activeRun, "An inactive parent can still own a child request")
        assertEquals("child", facts.conversations.first().ownedTaskPermissions.single().getValue("child_conversation_id").jsonPrimitive.content)
        assertTrue(facts.hasMoreConversations)
        assertEquals(50, facts.nextConversationOffset)
        assertEquals("global-recent", facts.recentConversations.single().conversationId)
        assertTrue(facts.hasMoreQueuedInstances)
        assertEquals("occurrence", facts.queuedInstances.single().occurrenceId)
        assertNull(facts.queuedInstances.single().title, "Do not replace a missing exact-revision title with a newer definition")
    }

    @Test fun refreshReadsOffsetZeroRatherThanMergingDifferentSnapshotPages() {
        val offsets = mutableListOf<Int>()
        val reader = TodayPendingResponseReader { command ->
            offsets += command.getValue("input").jsonObject.getValue("conversation_offset").jsonPrimitive.int
            snapshot()
        }
        reader.read(); reader.read()
        assertEquals(listOf(0, 0), offsets)
    }

    @Test fun needsResponseIsTheCoreFlagRatherThanAClientPermissionHeuristic() {
        val original = snapshot()
        val conversations = original.getValue("conversations").jsonArray
        val fact = JsonObject(conversations.first().jsonObject + ("needs_response" to JsonPrimitive(false)))
        val wire = JsonObject(original + ("conversations" to JsonArray(listOf(fact))))
        val result = TodayPendingResponseReader { wire }.read()
        assertFalse(result.conversations.single().needsResponse)
        assertTrue(result.conversations.single().ownedTaskPermissions.isNotEmpty())
    }

    @Test fun failedOrMalformedReadCannotBecomeASuccessfulEmptySnapshot() {
        assertFailsWith<IllegalStateException> { TodayPendingResponseReader { error("Store read rejected") }.read() }
        val missingFlag = JsonObject(snapshot().filterKeys { it != "has_more_queued_instances" })
        assertFailsWith<SerializationException> { TodayPendingResponseReader { missingFlag }.read() }
        val unknownField = JsonObject(snapshot() + ("old_pending_alias" to JsonArray(emptyList())))
        assertFailsWith<SerializationException> { TodayPendingResponseReader { unknownField }.read() }
    }
}
