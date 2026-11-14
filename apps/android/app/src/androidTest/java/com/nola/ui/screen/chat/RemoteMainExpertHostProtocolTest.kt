// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.*
import com.nola.data.remote.api.*
import com.nola.domain.model.ModelInput
import com.nola.llm.provider.WebAgentProvider
import dagger.hilt.android.EntryPointAccessors
import java.io.File
import java.util.UUID
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** Actual encrypted Main source submission, Host dispatcher and canonical journal mirror.
 * Opt-in disposable Host/AVD fixture only; deterministic model transport is not real-provider evidence. */
class RemoteMainExpertHostProtocolTest {
    @Test fun mainRunUsesSelectedExpertAndMirrorsOneUserAndRealTaskResult() = runBlocking<Unit> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val fixture = Json.parseToJsonElement(File(context.filesDir, "remote-main-expert-fixture.json").readText()).jsonObject
        require(fixture["disposableFixture"] == JsonPrimitive(true))
        val base = fixture.getValue("url").jsonPrimitive.content; require(java.net.URI(base).host == "127.0.0.1")
        val host = fixture.getValue("hostId").jsonPrimitive.content
        val parent = fixture.getValue("parent").jsonPrimitive.content
        val agent = fixture.getValue("agentId").jsonPrimitive.content
        val client = "android-main-expert-${UUID.randomUUID()}"
        val config = WebAgentProvider.Config(base, fixture.getValue("token").jsonPrimitive.content, expectedHostId = host, clientId = client)
        val provider = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java).webAgentProvider()
        val privateDirectory = File(context.cacheDir, "main-expert-${UUID.randomUUID()}").apply { check(mkdir()) }
        java.nio.file.Files.setPosixFilePermissions(privateDirectory.toPath(), setOf(java.nio.file.attribute.PosixFilePermission.OWNER_READ, java.nio.file.attribute.PosixFilePermission.OWNER_WRITE, java.nio.file.attribute.PosixFilePermission.OWNER_EXECUTE))
        AndroidApplicationRuntime(context, File(privateDirectory, "state.db")).use { runtime ->
            runtime.createConversation(parent, "Owned remote Main", 1, 1, buildJsonObject { put("executionHostBinding", buildJsonObject { put("baseURL", base); put("tlsCertificateSHA256", JsonNull) }) }.toString())
            val directory = provider.expertRoomReply(buildJsonObject { put("type", "directory") }, config, null)
            val epoch = directory.getValue("source_store_epoch").jsonPrimitive.content
            assertEquals(fixture.getValue("epoch").jsonPrimitive.content, epoch)
            val selectedAgent = directory.getValue("result").jsonObject.getValue("agents").jsonArray.map { it.jsonObject }.single { it["id"] == JsonPrimitive(agent) }
            try {
                provider.expertRoomReply(buildJsonObject { put("type", "ensure_for_conversation"); put("conversation_id", parent); put("at_ms", 1) }, config, "foreign-epoch")
                fail("Wrong source epoch must not mutate the Room")
            } catch (failure: WebHostStatusException) { assertEquals(409, failure.statusCode); assertEquals("room_source_epoch_changed", failure.errorCode) }
            val selections = buildJsonArray { add(buildJsonObject { put("agent_id", agent); put("agent_revision", selectedAgent.getValue("revision")); put("participant_id", JsonNull); put("context_conversation_id", JsonNull) }) }
            // Wrong directory epoch must reject in Begin without user/Run/Task or a partial Room join.
            AndroidApplicationRuntime(context, File(privateDirectory, "rejected.db")).use { rejected ->
                rejected.createConversation(parent, "Wrong-source Main", 1, 1, buildJsonObject { put("executionHostBinding", buildJsonObject { put("baseURL", base); put("tlsCertificateSHA256", JsonNull) }) }.toString())
                val wrongRun = UUID.randomUUID().toString(); val wrongText = "Wrong source must not run."
                val prepared = rejected.prepareRemoteSubmission(host, client, parent, "Wrong-source Main", wrongRun, wrongText, null, System.currentTimeMillis())
                val wrongRequest = provider.prepareAgentRequest(ModelInput(messages = listOf(mapOf("role" to "user", "content" to wrongText))), parent,
                    submissionId = wrongRun, configuration = config, currentUserDisplayText = wrongText,
                    expectedConfigurationRevision = fixture["configurationRevision"]?.jsonPrimitive?.longOrNull, selectedExperts = selections, expertSourceEpoch = "foreign-epoch")
                val wire = provider.freezeHostToolPlan(wrongRequest, config)
                val frozenIntent = rejected.bindRemoteSubmissionRequest(prepared.identity, wire)
                val frozen = FrozenRemoteSubmission(frozenIntent.identity, checkNotNull(frozenIntent.requestDigest), wire)
                withTimeout(30_000) { try { provider.streamFrozenSubmission(frozen, config).collect { } }
                    catch (failure: Exception) { if (failure is kotlinx.coroutines.CancellationException) throw failure } }
                var definitive = false
                withTimeout(30_000) { while (!definitive) {
                    val source = provider.submissionTransport(config)(frozen.reconciliationRequest()).getValue("result")
                    if (source != JsonNull) {
                        val result = decodeRemoteSubmissionReceipt(source.jsonObject, frozen.identity, frozen.requestDigest)
                        assertTrue(result.phase != RemoteSubmissionPhase.Admitted)
                        definitive = result.phase == RemoteSubmissionPhase.Rejected
                    }
                    if (!definitive) delay(100)
                } }
                val association = provider.expertRoomReply(buildJsonObject { put("type", "for_conversation"); put("conversation_id", parent) }, config, epoch)
                assertEquals(JsonNull, association["result"])
            }
            val run = UUID.randomUUID().toString(); val text = "Use the selected expert for a bounded check, then give one unified answer."
            val intent = runtime.prepareRemoteSubmission(host, client, parent, "Owned remote Main", run, text, null, System.currentTimeMillis())
            val request = provider.prepareAgentRequest(ModelInput(messages = listOf(mapOf("role" to "user", "content" to text))), parent,
                submissionId = run, configuration = config, currentUserDisplayText = text,
                expectedConfigurationRevision = fixture["configurationRevision"]?.jsonPrimitive?.longOrNull, selectedExperts = selections, expertSourceEpoch = epoch)
            val frozenRequest = provider.freezeHostToolPlan(request, config)
            val bound = runtime.bindRemoteSubmissionRequest(intent.identity, frozenRequest)
            val frozen = FrozenRemoteSubmission(bound.identity, checkNotNull(bound.requestDigest), frozenRequest)
            withTimeout(120_000) { provider.streamFrozenSubmission(frozen, config).collect { } }
            val key = RemoteRunKey(parent, run, "$run:assistant")
            val journal = AndroidAssistantJournal(runtime)
            var terminal = false
            withTimeout(30_000) {
                while (!terminal) {
                    val raw = provider.observationTransport(config)(remoteObservationRequest(host, client, parent, key))
                    val observation = checkNotNull(decodeRemoteRunObservation(raw, host, client, parent, key))
                    val source = observation.submissionReceipt
                    if (source != null) {
                        runtime.acceptRemoteSubmissionReceipt(decodeObservedSubmissionReceipt(source, host, client, key))
                        terminal = journal.catchUpRun(AndroidJournalTurn(host, parent, run, "$run:assistant", run), provider.journalTransport(config), { true }, observation.admission, observation.terminalSeal).second
                    }
                    if (!terminal) delay(100)
                }
            }
            // Use the same bounded window + canonical body reader as ConversationRepository.
            // The raw projection intentionally carries journal references rather than legacy text copies.
            val window=runtime.loadConversationProjectionWindowV1(parent,null,80,512_000)
            val heads=window.assistantJournalHeads.filter { cursor -> val owner=cursor.getValue("identity").jsonObject
                owner["source_store_epoch"]==JsonPrimitive(epoch)&&owner["conversation_id"]==JsonPrimitive(parent)&&owner["run_id"]==JsonPrimitive(run)
            }
            assertTrue("A sealed Main mirror must retain its exact source journal heads",heads.isNotEmpty())
            val projection=journal.overlay(window.projection,heads)
            File(context.filesDir,"remote-main-mirror-projection.json").writeText(buildJsonObject {
                put("heads",JsonArray(heads));put("messages",buildJsonArray {projection.messages.forEach { message ->add(buildJsonObject {
                    put("id",message.messageId);put("role",message.role.name);put("text",message.parts.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>().joinToString("\n") {it.markdown})
                }) }})
            }.toString())
            assertEquals(1, projection.messages.count { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User })
            assertTrue(projection.messages.any { it.messageId == "$run:assistant" && it.parts.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>().any { part -> part.markdown.contains("source-finished") } })
            assertEquals(ConversationSourceOwner.Remote(host, epoch), runtime.conversationSourceOwner(parent))
            val children = provider.readExpertHistoryCommand(buildJsonObject { put("type", "child_conversations"); put("conversation_id", parent); put("limit", 40); put("offset", 0) }, config, epoch)
            val child = children.getValue("children").jsonArray.map { it.jsonObject }.single { it["agent_id"] == JsonPrimitive(agent) }
            val history = provider.readExpertHistoryCommand(buildJsonObject { put("type", "child_history"); put("conversation_id", parent); put("child_conversation_id", child.getValue("conversation_id")); put("before_ordinal", JsonNull); put("limit", 50); put("content_byte_limit", 262144) }, config, epoch)
            assertTrue(history.toString().contains("target-own-high"))
            val exchanges = history.getValue("expert_exchanges").jsonArray
            assertEquals(1, exchanges.size)
            assertEquals(JsonPrimitive("main_agent"), exchanges.single().jsonObject.getValue("source").jsonObject["kind"])
            assertEquals(JsonPrimitive(parent), exchanges.single().jsonObject.getValue("source").jsonObject["context_conversation_id"])
            File(context.filesDir, "remote-main-expert-result.json").writeText(buildJsonObject { put("passed", true); put("parent", parent); put("run", run); put("child", child.getValue("conversation_id")); put("host", host); put("parentUsers", 1); put("mainJournalSealed", terminal) }.toString())
        }
    }
}
