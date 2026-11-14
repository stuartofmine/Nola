// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.core.*
import com.nola.data.local.*
import com.nola.data.repository.*
import com.nola.domain.model.ModelParameters
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

/** Exercises the actual Android Main adapter, factory, common Task worker and own target model.
 * Explicit disposable AVD and deterministic HTTP fixture required. No mock coordinator/receipt. */
class AndroidExpertNativeTaskIntegrationTest {
    @Test fun mainAgentUsesExistingTaskPumpAndExpertLeafOwnHighModel() = runFixture("complete")
    @Test fun sameMainTwoRunsReuseProviderCallIdsWithoutCrossBinding() = runFixture("repeat")
    @Test fun idleStreamingExpertCancellationDrainsExactStop() = runFixture("idle")
    @Test fun busyStreamingExpertCancellationDrainsExactStop() = runFixture("busy")

    private fun runFixture(mode: String) = runBlocking<Unit> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val fixture = Json.parseToJsonElement(File(context.filesDir, "native-expert-host-fixture.json").readText()).jsonObject
        require(fixture["disposableFixture"] == JsonPrimitive(true))
        require((fixture["mode"]?.jsonPrimitive?.contentOrNull ?: "complete") == mode)
        val endpoint = fixture.getValue("endpoint").jsonPrimitive.content
        require(java.net.URI(endpoint).host == "127.0.0.1")
        val dependencies = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java)
        val runtime = dependencies.runtime(); val service = dependencies.turnService(); val rooms = AgentRoomRepository(runtime)
        val prefix = UUID.randomUUID().toString()
        val contextTokens = if (mode == "repeat") 131072 else 32768
        for (name in listOf("source", "target")) runtime.upsertModelProfile(CanonicalModelProfile(
            "$prefix-$name", "Native $name", "openai-compatible", "$name-model", "remote",
            buildJsonObject {
                put("baseURL", endpoint); put("wireProtocol", "responses"); put("credentialRequired", false)
                put("capabilities", buildJsonArray { add("chat") }); put("executionCapabilities", buildJsonObject { put("toolCalls", true); put("imageInput", false); put("reasoning", true); put("samplingParameters", false) })
                put("reasoning", buildJsonObject { put("control", "effort"); put("efforts", buildJsonArray { add("low"); add("high") }) })
            }.toString(), """{"maxContextLength":$contextTokens,"maxOutputTokens":1024}""", name == "source", true, 0, 1, 1))
        var directory = rooms.directory()
        directory = rooms.createAgent("Native target $prefix", buildJsonObject {
            put("modelSelectionID", "configured:$prefix-target"); put("expertReasoningEffort", "high")
        }, directory, "Owned expert leaf fixture")
        val targetAgent = directory.agents.single { it.name == "Native target $prefix" }
        val parent = UUID.randomUUID().toString()
        runtime.createConversation(parent, "Owned Main delegation", 1, 1, buildJsonObject { put("modelSelectionID", "configured:$prefix-source") }.toString())
        val completedRounds = mutableListOf<JsonObject>()
        val rounds = if (mode == "repeat") 2 else 1
        for (round in 1..rounds) {
            val runID = UUID.randomUUID().toString(); val message = UUID.randomUUID().toString()
            val originalText = "Perform bounded Main delegation $round"
            File(context.filesDir, "native-expert-active-run.json").writeText(buildJsonObject { put("run", runID); put("round", round) }.toString())
            val selection = mainExpertSelection(setOf(targetAgent.id), mapOf(targetAgent.id to targetAgent.revision), emptyMap())
            val contextOwner = checkNotNull(runtime.loadConversation(parent))
            val expectedParent = runtime.loadConversationProjectionV1(parent).messages.lastOrNull()?.messageId
            val model = dependencies.modelRepository().listConfiguredModels().single { it.profileId == "$prefix-source" }
            val parameters = ModelParameters(maxCompletionTokens = 1024, maxContextTokens = contextTokens, reasoningPreference = "low")
            if (round == 1) assertNull("Selecting an expert must not create a Room before send", rooms.forConversation(parent))
            val plan = service.toolPlan(parent, true)
            val handle = service.start(AndroidAgentTurnRequest(service.captureModel(model, parameters), parent,
                originalText, tools = plan.tools, toolPlan = plan,
                canonicalTurn = AndroidCanonicalTurnDescriptor(parent, "Main", runID,
                    message, originalText, null, expectedConfigurationRevision = contextOwner.configurationRevision, expectedParentMessageId = expectedParent, mainExpertSelection = selection),
                resultEgressAuthorized = true, delegationEnabled = true))
            var terminal = false
            withTimeout(90_000) { handle.events.collect { event ->
                rooms.forConversation(parent)?.room?.room?.participants?.singleOrNull { it.agentId == targetAgent.id }?.let { target ->
                    File(context.filesDir, "native-expert-target.json").writeText(buildJsonObject { put("target", target.id); put("targetContext", target.contextConversationId); put("sourceContext", parent); put("parent", parent); put("sourceRun", runID) }.toString())
                }
                val tasks = runtime.execute(buildJsonObject { put("type", "runtime_task"); put("command", buildJsonObject { put("type", "list_all") }) }).jsonArray
                File(context.filesDir, "native-expert-tasks.json").writeText(tasks.toString())
                if (event is DeviceAgentCoordinatorEvent.Completed) terminal = true
                check(event !is DeviceAgentCoordinatorEvent.PermissionRequested) { "Same-parent expert orchestration unexpectedly requested permission" }
            } }
            assertTrue("Main did not produce a real terminal seal", terminal)
            val target = checkNotNull(rooms.forConversation(parent)).room.room.participants.single { it.agentId == targetAgent.id }
            val tasks = runtime.execute(buildJsonObject { put("type", "runtime_task"); put("command", buildJsonObject { put("type", "list_all") }) }).jsonArray.map { it.jsonObject }.filter { it["owner_session_id"] == JsonPrimitive(parent) }
            val mainSeal = runtime.executeQuery(buildJsonObject { put("type", "local_assistant_run_terminal_seal"); put("conversation_id", parent); put("run_id", runID) }).jsonObject
            File(context.filesDir, "native-expert-diagnostic.json").writeText(buildJsonObject {
                put("parent", parent); put("run", runID); put("mode", mode)
                put("tasks", JsonArray(tasks))
                put("mainSeal", mainSeal)
                put("projection", runtime.executeQuery(buildJsonObject { put("type", "load_conversation_projection_v1"); put("conversation_id", parent) }))
            }.toString())
            assertEquals(JsonPrimitive("answered"), mainSeal.getValue("result").jsonObject.getValue("value").jsonObject["kind"])
            assertEquals(round, tasks.size)
            val task = tasks.single { item ->
                Json.parseToJsonElement(item.getValue("metadata_json").jsonPrimitive.content).jsonObject
                    .getValue("expert_target").jsonObject["source_run_id"] == JsonPrimitive(runID)
            }
            assertEquals(JsonPrimitive(target.contextConversationId), task["child_session_id"])
            assertEquals(JsonPrimitive("sealed"), task["status"])
            val taskID = task.getValue("id").jsonPrimitive.content
            val childSeal = runtime.executeQuery(buildJsonObject {
                put("type", "local_assistant_run_terminal_seal"); put("conversation_id", target.contextConversationId); put("run_id", taskID)
            }).jsonObject
            assertEquals(JsonPrimitive(taskID), childSeal.getValue("identity").jsonObject["run_id"])
            assertEquals(JsonPrimitive(target.contextConversationId), childSeal.getValue("identity").jsonObject["conversation_id"])
            if (mode == "complete" || mode == "repeat") {
                assertTrue(runtime.loadConversationProjectionV1(target.contextConversationId).messages.any { item -> item.messageId == "$taskID:assistant" && item.parts.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>().any { it.markdown.contains("target-own-high") } })
            } else {
                val cancelled = childSeal.getValue("result").jsonObject.getValue("value").jsonObject
                assertEquals(JsonPrimitive("cancelled"), cancelled["kind"])
                assertEquals("Joined model producer must not leave unknown effects", JsonArray(emptyList()), cancelled["unknown_effect_refs"])
                assertTrue("Core did not persist requested cancellation", task["cancel_requested"] == JsonPrimitive(true))
                withTimeout(10_000) { while (AndroidEmbeddedChildExecutor.isActive(taskID)) delay(25) }
            }
            AndroidApplicationRuntime(context, runtime.databaseFile).use { reopened ->
                val restored = reopened.executeQuery(buildJsonObject { put("type", "runtime_task"); put("command", buildJsonObject { put("type", "get"); put("id", taskID) }) }).jsonObject
                assertEquals(JsonPrimitive("sealed"), restored["status"])
            }
            service.resumeEmbeddedChildren(); delay(500)
            completedRounds += buildJsonObject { put("run", runID); put("task", taskID); put("userMessage", message); put("text", originalText); put("childSeal", childSeal); put("mainSeal", mainSeal) }
        }
        val originalUsers = runtime.loadConversationProjectionV1(parent).messages.filter { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User }
        assertEquals(rounds, originalUsers.size)
        assertEquals(completedRounds.map { it.getValue("userMessage").jsonPrimitive.content }, originalUsers.map { it.messageId })
        assertEquals(rounds, completedRounds.map { it.getValue("task") }.distinct().size)
        assertEquals(completedRounds.map { it.getValue("text").jsonPrimitive.content }, originalUsers.map { item ->
            item.parts.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Text>().joinToString("") { it.markdown }
        })
        val childContext = completedRounds.first().getValue("childSeal").jsonObject.getValue("identity").jsonObject.getValue("conversation_id").jsonPrimitive.content
        val childUsers = runtime.loadConversationProjectionV1(childContext).messages.filter { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User }
        assertEquals(completedRounds.map { it.getValue("task").jsonPrimitive.content + ":user" }, childUsers.map { it.messageId })
        val last = completedRounds.last()
        File(context.filesDir, "native-expert-result.json").writeText(buildJsonObject {
            put("passed", true); put("parent", parent); put("source", parent); put("target", childContext)
            put("task", last.getValue("task")); put("terminal", true); put("mode", mode)
            put("childSeal", last.getValue("childSeal")); put("rounds", JsonArray(completedRounds))
        }.toString())

    }
}
