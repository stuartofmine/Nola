package com.nola.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreTaskProjectionTest {
    @Test fun resultStructureMatchesDurableBoundsAndDoesNotAllowEmptyOrConflictingEvidence() {
        val base = Json.parseToJsonElement("""{"result_id":"r","run_id":"run","conversation_id":"c","originating_submission_id":"s",
            "input":{"watermark":"1","applications_digest":"${"a".repeat(64)}"},"decided_at_revision":"1","decision_event_cursor":"2",
            "requirement_revision":"0","evidence_frontier":"0","value":{"kind":"answered","answer_part_refs":["a"]}}""").jsonObject
        assertTrue(base.hasValidRunResultStructure())
        for (variant in listOf(
            """{"kind":"blocked","reason":{"code":"blocked","detail":null},"blocking_fact_refs":[],"resolution_condition":"restore"}""",
            """{"kind":"incomplete","reason":{"code":"unfinished","detail":null},"remaining_requirement_refs":[],"obligation_refs":[]}""",
            """{"kind":"failed","error_ref":"error","observed_effect_refs":["effect"],"unknown_effect_refs":["effect"]}""",
            """{"kind":"blocked","reason":{"code":"Localized reason","detail":null},"blocking_fact_refs":["fact"],"resolution_condition":"restore"}""",
        )) assertFalse(JsonObject(base + ("value" to Json.parseToJsonElement(variant))).hasValidRunResultStructure())
        for (field in listOf("decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier")) {
            assertTrue(JsonObject(base + (field to kotlinx.serialization.json.JsonPrimitive(Long.MAX_VALUE.toString()))).hasValidRunResultStructure())
            assertFalse(JsonObject(base + (field to kotlinx.serialization.json.JsonPrimitive("9223372036854775808"))).hasValidRunResultStructure())
        }
        val empty = "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
        for ((watermark, digest) in listOf("0" to "a".repeat(64), "1" to empty, "9223372036854775808" to "a".repeat(64))) {
            assertFalse(JsonObject(base + ("input" to buildJsonObject { put("watermark", watermark); put("applications_digest", digest) })).hasValidRunResultStructure())
        }
        assertTrue(JsonObject(base + ("input" to buildJsonObject { put("watermark", "0"); put("applications_digest", empty) })).hasValidRunResultStructure())
    }
    @Test fun sealedAgentRetainsExactResultAndReceiptWithoutClaimingBusinessCompletion() {
        val running = update(cursor = 1, eventRevision = 1, projectionRevision = 1,
            state = AgentCoreTaskState.Running, progress = 0.25).let {
            it.copy(task = it.task.copy(kind = AgentCoreTaskKind.Agent))
        }
        val initial = AgentCoreTaskProjectionReducer.reduce(AgentCoreTaskProjection(), running)
        assertTrue(initial.applied)
        for (kind in listOf("answered", "action_completed", "delegation_accepted", "incomplete", "blocked", "cancelled", "failed")) {
            val evidence = when (kind) {
                "answered" -> "\"answer_part_refs\":[\"answer\"]"
                "action_completed" -> "\"requirement_acceptance_refs\":[\"assessment\"]"
                "delegation_accepted" -> "\"task_id\":\"task\",\"parent_call_id\":\"call\""
                "incomplete" -> "\"reason\":{\"code\":\"unfinished\",\"detail\":null},\"remaining_requirement_refs\":[\"requirement\"],\"obligation_refs\":[]"
                "blocked" -> "\"reason\":{\"code\":\"unavailable\",\"detail\":null},\"blocking_fact_refs\":[\"fact\"],\"resolution_condition\":\"Resource restored\""
                "cancelled" -> "\"cancellation_id\":\"cancel\",\"observed_effect_refs\":[],\"unknown_effect_refs\":[]"
                else -> "\"error_ref\":\"error\",\"observed_effect_refs\":[],\"unknown_effect_refs\":[]"
            }
            val result = Json.parseToJsonElement("""{
                "result_id":"decision","run_id":"child-run","conversation_id":"child-session-1",
                "originating_submission_id":"submission","input":{"watermark":"1","applications_digest":"${"b".repeat(64)}"},
                "decided_at_revision":"2","decision_event_cursor":"4","requirement_revision":"1","evidence_frontier":"3",
                "value":{"kind":"$kind",$evidence}
            }""").jsonObject
            val sealed = running.copy(cursor = 2, revision = 2, taskProjectionRevision = 2,
                task = running.task.copy(state = AgentCoreTaskState.Sealed, durability = AgentCoreTaskDurability(
                    runResult = result, resultDigest = "a".repeat(64), terminalReceiptId = "receipt")))
            val accepted = AgentCoreTaskProjectionReducer.reduce(initial.projection, sealed)
            assertTrue(accepted.applied)
            assertFalse(sealed.task.copy(lineage = sealed.task.lineage.copy(childSessionId = "foreign")).hasValidResultShape)
            assertFalse(sealed.task.copy(durability = sealed.task.durability.copy(
                runResult = JsonObject(result + ("value" to buildJsonObject { put("kind", kind) })))).hasValidResultShape)
            assertEquals(result, accepted.projection.tasks["task-1"]?.durability?.runResult)
            assertTrue(accepted.projection.tasks["task-1"]!!.state.isTerminal)
            val rawTask = buildJsonObject {
                put("id", "task-1"); put("owner_session_id", "session-1"); put("owner_turn_id", "turn-1")
                put("kind", "agent"); put("state", "sealed"); put("cancel_requested", false)
                put("lineage", buildJsonObject { put("child_session_id", "child-session-1") })
                put("durability", buildJsonObject { put("run_result", result); put("result_digest", "a".repeat(64)); put("terminal_receipt_id", "receipt") })
            }
            val restored = decodeAgentCoreTaskProjection(buildJsonObject {
                put("revision", 2); put("order", buildJsonArray { add("task-1") })
                put("tasks", buildJsonObject { put("task-1", rawTask) })
            }, "session-1")
            assertEquals(result, restored?.tasks?.get("task-1")?.durability?.runResult)
            assertFalse(AgentCoreTaskProjectionReducer.reduce(accepted.projection,
                running.copy(cursor = 3, revision = 3, taskProjectionRevision = 3)).applied)
            for (durability in listOf(sealed.task.durability.copy(runResult = null),
                sealed.task.durability.copy(resultDigest = null), sealed.task.durability.copy(terminalReceiptId = null))) {
                assertFalse(AgentCoreTaskProjectionReducer.reduce(initial.projection,
                    sealed.copy(task = sealed.task.copy(durability = durability))).applied)
            }
        }
        assertFalse(AgentCoreTaskProjectionReducer.reduce(initial.projection, running.copy(cursor = 2, revision = 2,
            taskProjectionRevision = 2, task = running.task.copy(state = AgentCoreTaskState.Completed))).applied)
    }
    @Test
    fun productionReducerRestoresAndAppliesOnlyAuthoritativeMonotonicUpdates() {
        val restored = decodeAgentCoreTaskProjection(
            taskProjectionJson(revision = 1, state = "running", progress = 0.25),
            expectedSessionId = "session-1",
        ) ?: error("projection must restore")
        assertEquals("task-1", restored.orderedTasks.single().id)
        assertEquals("call-parent-1", restored.orderedTasks.single().lineage.parentCallId)

        val completed = update(
            cursor = 8,
            eventRevision = 12,
            projectionRevision = 2,
            state = AgentCoreTaskState.Completed,
            progress = 1.0,
        )
        val applied = AgentCoreTaskProjectionReducer.reduce(restored, completed)
        assertTrue(applied.applied)
        assertEquals(AgentCoreTaskState.Completed, applied.projection.tasks["task-1"]?.state)
        assertFalse(AgentCoreTaskProjectionReducer.reduce(applied.projection, completed).applied)
        assertFalse(
            AgentCoreTaskProjectionReducer.reduce(
                restored,
                completed.copy(
                    taskProjectionRevision = 3,
                    task = completed.task.copy(ownerTurnId = "wrong-turn"),
                ),
            ).applied,
        )
        assertFalse(
            AgentCoreTaskProjectionReducer.reduce(
                restored,
                completed.copy(
                    taskProjectionRevision = 3,
                    task = completed.task.copy(progress = 0.1),
                ),
            ).applied,
        )
        assertFalse(
            AgentCoreTaskProjectionReducer.reduce(
                applied.projection,
                completed.copy(
                    cursor = 9,
                    revision = 13,
                    taskProjectionRevision = 3,
                    task = completed.task.copy(state = AgentCoreTaskState.Running),
                ),
            ).applied,
        )
    }

    @Test
    fun decoderIgnoresOrdinaryToolEventsAndRejectsWrongSessionCheckpoint() {
        val shortToolEnvelope = buildJsonObject {
            put("session_id", "session-1")
            put("revision", 1)
            put("cursor", buildJsonObject { put("sequence", 1) })
            put("event", buildJsonObject {
                put("type", "part_updated")
                put("turn_id", "turn-1")
            })
        }
        assertNull(decodeAgentCoreTaskUpdate(shortToolEnvelope))
        assertNull(
            decodeAgentCoreTaskProjection(
                taskProjectionJson(revision = 1, state = "running", progress = 0.25),
                expectedSessionId = "another-session",
            ),
        )
    }

    @Test
    fun eventDecoderAndCancellationSubmissionMatchCoreWireContract() {
        val event = buildJsonObject {
            put("session_id", "session-1")
            put("turn_id", "turn-1")
            put("revision", 9)
            put("cursor", buildJsonObject { put("sequence", 7) })
            put("event", buildJsonObject {
                put("type", "task_updated")
                put("task_projection_revision", 2)
                put("task", taskJson("waiting", 0.5))
            })
        }
        val decoded = decodeAgentCoreTaskUpdate(event) ?: error("event must decode")
        assertEquals("session-1", decoded.task.ownerSessionId)
        assertEquals("turn-1", decoded.task.ownerTurnId)
        assertEquals(2, decoded.taskProjectionRevision)

        val submission = agentCoreTaskCancellationSubmission("task-1")
        assertEquals("request_task_cancellation", submission["type"]?.jsonPrimitive?.content)
        assertEquals("task-1", submission["task_id"]?.jsonPrimitive?.content)
    }

    private fun update(
        cursor: Long,
        eventRevision: Long,
        projectionRevision: Long,
        state: AgentCoreTaskState,
        progress: Double,
    ) = AgentCoreTaskUpdate(
        sessionId = "session-1",
        turnId = "turn-1",
        cursor = cursor,
        revision = eventRevision,
        taskProjectionRevision = projectionRevision,
        task = AgentCoreTaskHandle(
            id = "task-1",
            ownerSessionId = "session-1",
            ownerTurnId = "turn-1",
            kind = AgentCoreTaskKind.Skill,
            state = state,
            progress = progress,
            currentStep = if (state == AgentCoreTaskState.Completed) "done" else "rendering",
            resultRef = if (state == AgentCoreTaskState.Completed) "artifact://result.docx" else null,
            lineage = AgentCoreTaskLineage(
                childSessionId = "child-session-1",
                parentCallId = "call-parent-1",
                permissionScopeId = "conversation:session-1",
                cancellationScopeId = "task:task-1",
            ),
        ),
    )

    private fun taskProjectionJson(
        revision: Long,
        state: String,
        progress: Double,
    ): JsonObject = buildJsonObject {
        put("revision", revision)
        put("order", buildJsonArray { add("task-1") })
        put("tasks", buildJsonObject { put("task-1", taskJson(state, progress)) })
    }

    private fun taskJson(state: String, progress: Double): JsonObject = buildJsonObject {
        put("id", "task-1")
        put("owner_session_id", "session-1")
        put("owner_turn_id", "turn-1")
        put("kind", "skill")
        put("state", state)
        put("progress", progress)
        put("current_step", "rendering")
        put("lineage", buildJsonObject {
            put("child_session_id", "child-session-1")
            put("parent_call_id", "call-parent-1")
            put("permission_scope_id", "conversation:session-1")
            put("cancellation_scope_id", "task:task-1")
        })
        put("cancel_requested", false)
    }
}
