package com.nola.data.repository

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ConversationProgressRepositoryTest {
    private fun projection(token: JsonElement = JsonPrimitive("opaque-token")): JsonObject = buildJsonObject {
        put("can_pause",false); put("can_resume",true); put("can_cancel",true)
        put("revision",3); put("control_generation",2); put("control_token",token)
        put("active",buildJsonObject {
            put("id","goal"); put("objective","Ship"); put("status","paused")
            put("requirements",buildJsonObject { put("r", buildJsonObject {
                put("id","r"); put("description","Validate"); put("status","pending")
            }) })
        })
        put("applied_operations",buildJsonObject { put("fixed-operation", buildJsonObject {
            put("command_sha256","a".repeat(64)); put("goal_revision",3); put("rejection",JsonNull)
        }) })
    }
    private fun remote(goal: JsonElement = projection()) = buildJsonObject {
        put("source_host_id","host"); put("conversation_id","a"); put("goal_projection",goal); put("plan",JsonNull)
    }
    @Test fun usesExplicitCoreGoalIdentityTokenAndOperationLedger() {
        val result = ConversationProgressRepository.decodeRemote("a","host",remote())
        assertEquals("paused",result.status)
        assertEquals("pending",result.requirements.single().status)
        val control = requireNotNull(result.goalControl)
        assertEquals("goal",control.goalId); assertEquals("opaque-token",control.controlToken)
        assertEquals(3uL,control.revision)
        assertFalse(control.canPause); assertTrue(control.canResume); assertTrue(control.canCancel)
        assertEquals("a".repeat(64),control.appliedOperations.getValue("fixed-operation").commandSha256)
        assertNull(control.appliedOperations.getValue("fixed-operation").rejection)
    }
    @Test fun pairedProgressRequiresExactHostConversationAndAuthorityFields() {
        val empty = remote(JsonNull)
        assertNull(ConversationProgressRepository.decodeRemote("a","host",empty).goalControl)
        assertThrows(IllegalArgumentException::class.java) { ConversationProgressRepository.decodeRemote("a","other",empty) }
        assertThrows(IllegalArgumentException::class.java) { ConversationProgressRepository.decodeRemote("other","host",empty) }
        assertThrows(IllegalArgumentException::class.java) { ConversationProgressRepository.decodeRemote("a","host",JsonObject(empty-"plan")) }
        for (key in listOf("control_token","revision","control_generation","applied_operations","can_pause","can_resume","can_cancel")) {
            assertThrows(RuntimeException::class.java) { ConversationProgressRepository.decodeRemote("a","host",remote(JsonObject(projection()-key))) }
        }
    }
    @Test fun localProgressNeverDerivesMissingControlToken() {
        val session = buildJsonObject { put("conversation_id","a"); put("goal_projection",JsonObject(projection()-"control_token")) }
        val result = ConversationProgressRepository.decode("a",session,JsonNull)
        assertEquals("goal",result.goalControl?.goalId); assertNull(result.goalControl?.controlToken)
        assertThrows(IllegalArgumentException::class.java) { ConversationProgressRepository.decode("other",session,JsonNull) }
    }
    @Test fun invalidGoalFactsAndLedgerDoNotBecomeControlAuthority() {
        val value = projection()
        val mutations = listOf(
            JsonObject(value + ("control_token" to JsonNull)),
            JsonObject(value + ("revision" to JsonPrimitive("3"))),
            JsonObject(value + ("control_generation" to JsonPrimitive(4))),
            JsonObject(value + ("active" to JsonObject(value.getValue("active").jsonObject + ("status" to JsonPrimitive("unknown"))))),
            JsonObject(value + ("active" to JsonObject(value.getValue("active").jsonObject - "id"))),
            JsonObject(value + ("applied_operations" to buildJsonObject { put("fixed-operation",buildJsonObject {
                put("command_sha256","bad"); put("goal_revision",3); put("rejection",JsonNull)
            }) }))
        )
        mutations.forEach { malformed -> assertThrows(RuntimeException::class.java) {
            ConversationProgressRepository.decodeRemote("a","host",remote(malformed))
        } }
    }
    @Test fun capabilitiesAreAuthoritativeEvenWhenStatusHasNotChanged() {
        val raw = projection()
        val allowed = JsonObject(raw + ("can_pause" to JsonPrimitive(true)))
        assertTrue(ConversationProgressRepository.decodeRemote("a", "host", remote(allowed)).goalControl!!.canPause)
        assertThrows(RuntimeException::class.java) {
            ConversationProgressRepository.decodeRemote("a", "host", remote(JsonObject(raw + ("can_resume" to JsonPrimitive("true")))))
        }
        val session = buildJsonObject { put("conversation_id", "a"); put("goal_projection", raw) }
        val observation = buildJsonObject {
            put("goal_id", "goal"); put("status", "paused"); put("control_token", "observed")
            put("can_pause", true); put("can_resume", false); put("can_cancel", true)
        }
        val local = ConversationProgressRepository.withLocalControlObservation(session, observation)
        val control = ConversationProgressRepository.decode("a", local, JsonNull).goalControl!!
        assertEquals("observed", control.controlToken); assertTrue(control.canPause); assertFalse(control.canResume)
        assertThrows(IllegalArgumentException::class.java) {
            ConversationProgressRepository.withLocalControlObservation(session, JsonObject(observation + ("goal_id" to JsonPrimitive("foreign"))))
        }
    }
    @Test fun rejectionIsPreservedAndPlanStatusIsNotInferred() {
        val value = projection()
        val rejected = JsonObject(value + ("applied_operations" to buildJsonObject { put("fixed-operation",buildJsonObject {
            put("command_sha256","b".repeat(64)); put("goal_revision",3); put("rejection","stale_control")
        }) }))
        assertEquals("stale_control",ConversationProgressRepository.decodeRemote("a","host",remote(rejected))
            .goalControl?.appliedOperations?.get("fixed-operation")?.rejection)
        val plan=Json.parseToJsonElement("""{"conversation_id":"a","todos":[{"id":"p","content":"Run checks","status":"blocked"}]}""")
        assertEquals("blocked",ConversationProgressRepository.decode("a",JsonNull,plan).todos.single().status)
        assertThrows(IllegalArgumentException::class.java) { ConversationProgressRepository.decode("other",JsonNull,plan) }
    }
}
