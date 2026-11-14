// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.*
import com.nola.data.remote.api.*
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.File
import java.util.UUID
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** Opt-in, owned emulator and disposable authenticated Host only; the Host dispatches real workers.
 * A deterministic provider is suitable for protocol acceptance, never a real-model certificate. */
class RemoteExpertHostProtocolTest {
    @Test fun encryptedAdmissionMirrorsCanonicalParentAndObservedChildJournal() = runBlocking<Unit> {
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
        val privateDirectory = File(context.cacheDir, "expert-protocol-${UUID.randomUUID()}").apply { check(mkdir()) }
        java.nio.file.Files.setPosixFilePermissions(privateDirectory.toPath(), setOf(java.nio.file.attribute.PosixFilePermission.OWNER_READ, java.nio.file.attribute.PosixFilePermission.OWNER_WRITE, java.nio.file.attribute.PosixFilePermission.OWNER_EXECUTE))
        val database = File(privateDirectory, "state.db")
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
            val admitted = mobile("chat/expert-messages/admit", envelope(buildJsonObject { put("input", input) }))
            val receipt = checkNotNull(decodeRemoteExpertAdmission(admitted, host, client, parent, message))
            AndroidApplicationRuntime(context, database).use { runtime ->
                runtime.createConversation(parent, "Owned remote parent", 1, 1, buildJsonObject { put("executionHostBinding", buildJsonObject { put("baseURL", base); put("tlsCertificateSHA256", JsonNull) }) }.toString())
                val receive = buildJsonObject { put("type", "receive_remote_expert_parent_admission"); put("expected_source_host_id", host); put("expected_source_store_epoch", epoch); put("admission", receipt.parentAdmission) }
                runtime.execute(receive); runtime.execute(receive)
                assertEquals(1, runtime.loadConversationProjectionV1(parent).messages.size)
                assertEquals(ConversationSourceOwner.Remote(host, epoch), runtime.conversationSourceOwner(parent))
                val entry = receipt.manifest.entries.single()
                val child = entry.participant.contextConversationId
                val roomClient = "room:${receipt.manifest.input.roomId}"
                val key = RemoteRunKey(child, entry.childSubmissionId, entry.childAssistantMessageId)
                val journal = AndroidAssistantJournal(runtime)
                val deadline = android.os.SystemClock.elapsedRealtime() + 90_000
                var sealed = false
                while (!sealed && android.os.SystemClock.elapsedRealtime() < deadline) {
                    val raw = mobile("chat/agent/v2/observe", remoteObservationRequest(host, roomClient, child, key))
                    val observed = checkNotNull(decodeRemoteRunObservation(raw, host, roomClient, child, key))
                    val source = observed.submissionReceipt
                    if (source != null) {
                        runtime.adoptObservedRemoteRun(decodeObservedSubmissionReceipt(source, host, roomClient, key))
                        sealed = journal.catchUpRun(AndroidJournalTurn(host, child, entry.childSubmissionId, entry.childAssistantMessageId, entry.childSubmissionId),
                            { request -> mobile("chat/agent/v2/journal", JsonObject(request + ("expected_host_id" to JsonPrimitive(host)))) }, { true }, observed.admission, observed.terminalSeal).second
                    }
                    if (!sealed) delay(200)
                }
                assertTrue("Actual Host child did not seal", sealed)
                val childProjection = runtime.loadConversationProjectionV1(child)
                assertTrue(childProjection.messages.any { it.messageId == entry.childAssistantMessageId && it.parts.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>().any { part -> part.markdown.contains("REMOTE_EXPERT_PROTOCOL_OK") } })
                assertEquals(1, runtime.loadConversationProjectionV1(parent).messages.size)
                android.database.sqlite.SQLiteDatabase.openDatabase(database.path, null, android.database.sqlite.SQLiteDatabase.OPEN_READONLY).use { sql ->
                    sql.rawQuery("SELECT COUNT(*) FROM runs WHERE conversation_id=?", arrayOf(parent)).use { rows -> check(rows.moveToFirst()); assertEquals(0L, rows.getLong(0)) }
                    sql.rawQuery("SELECT COUNT(*) FROM runs WHERE conversation_id=?", arrayOf(child)).use { rows -> check(rows.moveToFirst()); assertEquals(1L, rows.getLong(0)) }
                }
                val reread = mobile("chat/expert-messages/manifest", envelope(buildJsonObject { put("conversation_id", parent); put("message_id", message) }))
                val retry = checkNotNull(decodeRemoteExpertAdmission(reread, host, client, parent, message))
                runtime.execute(JsonObject(receive + ("admission" to retry.parentAdmission)))
                File(context.filesDir, "remote-expert-host-result.json").writeText(buildJsonObject { put("passed", true); put("parent", parent); put("child", child); put("host", host); put("sourceEpoch", epoch); put("parentMessageCount", 1); put("childJournalSealed", sealed) }.toString())
            }
        }
    }
}
