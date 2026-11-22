// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.*
import com.nola.data.remote.api.*
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** New expert requests are owned by Main. Historical receipt reads are bound to the original submission. */
class RemoteExpertHostProtocolTest {
    @Test fun freshExpertOnlySubmissionIsDefinitivelyRejectedWithoutCreatingAReceipt() = runBlocking<Unit> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val fixtureFile = File(context.filesDir, "remote-expert-host-fixture.json")
        require(fixtureFile.isFile) { "Explicit disposable Host fixture required" }
        val fixture = Json.parseToJsonElement(fixtureFile.readText()).jsonObject
        require(fixture["disposableFixture"] == JsonPrimitive(true))
        val base = fixture.getValue("url").jsonPrimitive.content
        require(java.net.URI(base).host == "127.0.0.1")
        val token = fixture.getValue("token").jsonPrimitive.content
        val host = fixture.getValue("hostId").jsonPrimitive.content
        val agent = fixture.getValue("agentId").jsonPrimitive.content
        val agentRevision = fixture.getValue("agentRevision").jsonPrimitive.long
        val client = "android-expert-protocol-${UUID.randomUUID()}"
        val parent = UUID.randomUUID().toString()
        HttpClient(CIO).use { http ->
            suspend fun mobile(path: String, body: JsonObject) = postEncryptedWebHostRequest(http, "$base/api/mobile/$path", token, body,
                maximumRequestBytes = 4 * 1024 * 1024, maximumResponseBytes = 8 * 1024 * 1024)
            fun envelope(body: JsonObject) = JsonObject(body + mapOf("expected_host_id" to JsonPrimitive(host), "client_id" to JsonPrimitive(client)))
            val prepared = mobile("chat/expert-messages/parent", envelope(buildJsonObject { put("conversation_id", parent); put("title", "Owned Android remote expert") }))
            assertEquals(host, prepared.getValue("host_id").jsonPrimitive.content)
            val epoch = prepared.getValue("source_store_epoch").jsonPrimitive.content
            val association = prepared.getValue("room").jsonObject
            val room = association.getValue("room").jsonObject
            val joinCommand = buildJsonObject { put("type", "join"); put("input", buildJsonObject {
                put("room_id", room.getValue("room").jsonObject.getValue("id")); put("expected_room_revision", room.getValue("revision"))
                put("agent_id", agent); put("expected_agent_revision", agentRevision); put("at_ms", System.currentTimeMillis())
            }) }
            try {
                mobile("agent-rooms", buildJsonObject { put("expected_host_id", host); put("expected_source_store_epoch", "foreign-epoch"); put("command", joinCommand) })
                fail("A restored Store must not execute the old-source join")
            } catch (rejection: WebHostStatusException) { assertEquals(409, rejection.statusCode); assertEquals("room_source_epoch_changed", rejection.errorCode) }
            val joinedReply = mobile("agent-rooms", buildJsonObject {
                put("expected_host_id", host); put("expected_source_store_epoch", epoch)
                put("command", buildJsonObject { put("type", "join"); put("input", buildJsonObject {
                    put("room_id", room.getValue("room").jsonObject.getValue("id")); put("expected_room_revision", room.getValue("revision"))
                    put("agent_id", agent); put("expected_agent_revision", agentRevision); put("at_ms", System.currentTimeMillis())
                }) })
            })
            assertEquals(epoch, joinedReply.getValue("source_store_epoch").jsonPrimitive.content)
            val joined = joinedReply.getValue("result").jsonObject
            val member = joined.getValue("room").jsonObject.getValue("participants").jsonArray.map { it.jsonObject }.single { it["agent_id"] == JsonPrimitive(agent) }
            val message = UUID.randomUUID().toString()
            val input = buildJsonObject {
                put("conversation_id", parent); put("expected_revision", prepared.getValue("parent").jsonObject.getValue("revision"))
                put("room_id", joined.getValue("room").jsonObject.getValue("id")); put("expected_room_revision", joined.getValue("revision"))
                put("submission_id", message); put("user_message_id", message); put("text", "Return REMOTE_EXPERT_PROTOCOL_OK.")
                put("created_at_ms", System.currentTimeMillis()); put("selected_participants", buildJsonArray { add(buildJsonObject {
                    put("participant_id", member.getValue("id")); put("agent_id", member.getValue("agent_id")); put("agent_revision", member.getValue("agent_revision")); put("context_conversation_id", member.getValue("context_conversation_id"))
                }) })
            }
            val typedInput = Json.decodeFromJsonElement<AndroidAdmitExpertMessage>(input)
            val rejection = try {
                mobile("chat/expert-messages/admit", envelope(buildJsonObject { put("input", input) }))
                error("A fresh user submission must enter through the main agent")
            } catch (failure: WebHostStatusException) { failure }
            assertTrue(definitiveRemoteExpertRejection(rejection, host, client, typedInput))
            assertFalse(definitiveRemoteExpertRejection(rejection, host, "$client-other", typedInput))
            val manifest = mobile("chat/expert-messages/manifest", envelope(buildJsonObject {
                put("conversation_id", parent)
                put("message_id", message)
            }))
            assertNull(decodeRemoteExpertAdmission(manifest, host, client, parent, message))
            val unchanged = mobile("chat/expert-messages/parent", envelope(buildJsonObject {
                put("conversation_id", parent)
                put("title", "Owned Android remote expert")
            }))
            assertEquals(prepared.getValue("parent"), unchanged.getValue("parent"))
            File(context.filesDir, "remote-expert-host-result.json").writeText(buildJsonObject {
                put("passed", true)
                put("parent", parent)
                put("sourceEpoch", epoch)
                put("status", rejection.statusCode)
                put("code", rejection.errorCode)
                put("definitiveIdentityMatches", true)
                put("receiptAbsent", true)
            }.toString())
        }
    }
}
