// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test

class AgentRoomRepositoryTest {
    @Test fun canonicalExpertSubmissionIdentityIsStableAndParticipantScoped() {
        val first = canonicalExpertSubmissionId("c", "m", "p1")
        assertEquals(first, canonicalExpertSubmissionId("c", "m", "p1"))
        assertNotEquals(first, canonicalExpertSubmissionId("c", "m", "p2"))
        assertTrue(java.util.UUID.fromString(first).version() == 3)
    }

    @Test(expected = IllegalArgumentException::class)
    fun canonicalParentAndOwnerMustBePaired() {
        RoomSubmissionInput("r", "s", 1, "text", null, 1, parentMessageId = "m")
    }

    @Test fun conversationAssociationUsesSharedEnsureAndReadCommands() {
        val association = """{"conversation_id":"conversation","room_id":"room","revision":2,"room":{"revision":2,"room":{"id":"room","title":"Hidden","participants":[]}}}"""
        val repository = AgentRoomRepository { envelope ->
            val command = envelope["command"]!!.jsonObject
            when (command["type"]!!.jsonPrimitive.content) {
                "ensure_for_conversation" -> {
                    assertEquals("conversation", command["conversation_id"]!!.jsonPrimitive.content)
                    Json.parseToJsonElement(association)
                }
                "for_conversation" -> {
                    assertEquals("conversation", command["conversation_id"]!!.jsonPrimitive.content)
                    Json.parseToJsonElement(association)
                }
                else -> error("unexpected command")
            }
        }
        assertEquals("room", repository.ensureForConversation("conversation").roomId)
        assertEquals("conversation", repository.forConversation("conversation")?.conversationId)
    }

    @Test fun expertSubmissionCarriesCanonicalParentAndOwnerTogether() {
        val input = RoomSubmissionInput("room", "submission", 3, "answer this", "participant", 10,
            parentMessageId = "canonical-user", conversationId = "conversation")
        val repository = AgentRoomRepository { envelope ->
            val command = envelope["command"]!!.jsonObject
            assertEquals("submit", command["type"]!!.jsonPrimitive.content)
            val wire = command["input"]!!.jsonObject
            assertEquals("canonical-user", wire["parent_message_id"]!!.jsonPrimitive.content)
            assertEquals("conversation", wire["conversation_id"]!!.jsonPrimitive.content)
            Json.encodeToJsonElement(RoomSubmission.serializer(), RoomSubmission(input, "selected", "context",
                "participant", 1, null, null))
        }
        assertEquals(input, repository.submit(input).input)
    }

    @Test fun automaticRecipientRemainsNullAndClaimUsesRevision() {
        val input = RoomSubmissionInput("room", "submission", 3, "current 搜索 follow-up", null, 10)
        val encoded = """{"input":{"room_id":"room","submission_id":"submission","expected_room_revision":3,"text":"current 搜索 follow-up","participant_id":null,"at_ms":10},"state":"accepted","context_conversation_id":null,"selected_participant_id":null,"revision":1,"failure_code":null,"execution_id":null,"terminal":null}"""
        val repository = AgentRoomRepository { envelope ->
            val command = envelope["command"]!!.jsonObject
            assertEquals("submit", command["type"]!!.jsonPrimitive.content)
            assertEquals(JsonNull, command["input"]!!.jsonObject["participant_id"])
            Json.parseToJsonElement(encoded)
        }
        val accepted = repository.submit(input)
        assertNull(accepted.selectedParticipantId)
        AgentRoomRepository { envelope ->
            val command = envelope["command"]!!.jsonObject
            assertEquals("claim_execution", command["type"]!!.jsonPrimitive.content)
            assertEquals("1", command["expected_revision"]!!.jsonPrimitive.content)
            Json.parseToJsonElement("""{"submission":$encoded,"dispatch_permitted":false}""")
        }.claimExecution(accepted).also { assertFalse(it.dispatchPermitted) }
    }

    @Test fun terminalComesFromCoreObservationNotTransportDone() {
        val value = RoomSubmission(RoomSubmissionInput("room", "submission", 1, "hi", null, 1), "executing", "context", "p", 3, null, "run")
        val repository = AgentRoomRepository { envelope ->
            val command = envelope["command"]!!.jsonObject
            assertEquals("reconcile_execution", command["type"]!!.jsonPrimitive.content)
            assertNull(command["run_id"])
            assertNull(command["terminal"])
            Json.encodeToJsonElement(RoomSubmission.serializer(), value)
        }
        assertEquals("executing", repository.observeExecution(value).state)
    }
    @Test fun joinCarriesCapturedRoomAndAgentRevisions() {
        val room = AgentRoomSnapshot(3, AgentRoom("room", "Discussion", emptyList()))
        val agent = PersistentAgent("a", "A", 7, "active", buildJsonObject {}, "Research")
        val repository = AgentRoomRepository { envelope ->
            assertEquals("agent_room", envelope["type"]!!.jsonPrimitive.content)
            val command = envelope["command"]!!.jsonObject
            assertEquals("join", command["type"]!!.jsonPrimitive.content)
            val input = command["input"]!!.jsonObject
            assertEquals("room", input["room_id"]!!.jsonPrimitive.content)
            assertEquals("a", input["agent_id"]!!.jsonPrimitive.content)
            assertEquals("3", input["expected_room_revision"]!!.jsonPrimitive.content)
            assertEquals("7", input["expected_agent_revision"]!!.jsonPrimitive.content)
            assertNull(input["context_conversation_id"])
            Json.parseToJsonElement("""{"revision":3,"room":{"id":"room","title":"Discussion","participants":[]}}""")
        }
        assertEquals(room, repository.join(agent, room))
    }

    @Test fun absentRoomDoesNotCreateDefault() {
        assertNull(AgentRoomRepository { JsonNull }.load("missing"))
    }

    @Test(expected = IllegalStateException::class)
    fun failedMutationIsNotReportedAsSuccess() {
        AgentRoomRepository { error("room_revision_conflict") }.create("Discussion")
    }
}
