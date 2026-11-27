// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.scheduled

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.CanonicalModelProfile
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.ui.screen.chat.AndroidExpertNativeTestDependencies
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import com.nola.work.journalCompletedAt
import com.nola.work.journalSessionId
import dagger.hilt.android.EntryPointAccessors
import java.net.InetAddress
import java.net.ServerSocket
import java.net.SocketException
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Real WorkManager, Core, Room and MainActivity. The model and data source use loopback HTTP. */
class ScheduledPermissionOccurrenceTest {
    @get:Rule val compose = createEmptyComposeRule()

    @Test fun deniedOnceOccurrenceSurvivesOtherActionsRecentRuns() = runScenario(daily = false)
    @Test fun approvedRecurringOccurrenceSchedulesOneNextRun() = runScenario(daily = true)

    private fun runScenario(daily: Boolean) = runBlocking<Unit> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val dependencies = EntryPointAccessors.fromApplication(context, AndroidExpertNativeTestDependencies::class.java)
        val runtime = dependencies.runtime()
        val repo = dependencies.scheduledActions()
        val previousDefault = dependencies.preferences().defaultModelSelection(dependencies.modelRepository().listConfiguredModels())
        val prefix = UUID.randomUUID().toString()
        dependencies.modelService().usePlaceholder()
        ScheduledPermissionServer(daily).use { server ->
            val model = "scheduled-fixture"
            runtime.upsertModelProfile(CanonicalModelProfile(prefix, model, "openai-compatible", model, "remote",
                buildJsonObject {
                    put("baseURL", server.url + "/v1"); put("wireProtocol", "chat_completions"); put("credentialRequired", false)
                    put("capabilities", buildJsonArray { add("chat") })
                    put("executionCapabilities", buildJsonObject { put("toolCalls", true); put("imageInput", false); put("reasoning", false); put("samplingParameters", false) })
                }.toString(), """{"maxContextLength":32768,"maxOutputTokens":1024}""", true, true, 0, 1, 1))
            dependencies.preferences().saveDefaultModelSelection(dependencies.modelRepository().listConfiguredModels().single { it.profileId == prefix })
            val now = java.time.ZonedDateTime.now(java.time.ZoneOffset.UTC)
            val target = if (daily) {
                val nextMinute = now.plusMinutes(1).withSecond(0).withNano(0)
                if (java.time.Duration.between(now, nextMinute).seconds < 5) nextMinute.plusMinutes(1) else nextMinute
            } else now.plusSeconds(2)
            val scheduledAt = target.toInstant().toEpochMilli()
            val action = ScheduledActionEntity(prefix, "test", "Review scheduled permission", server.url + "/source",
                "json", "Inspect the clipboard for this disposable test.", scheduleFrequency = if (daily) "daily" else "once",
                scheduledTimeIso = if (daily) target.toLocalTime().toString() else java.time.Instant.ofEpochMilli(scheduledAt).toString())
            repo.create(action)
            val occurrence = "${action.id}:$scheduledAt"
            val resultId = "scheduled-result:$occurrence"
            val scheduler = dependencies.scheduledActionScheduler()
            try {
                scheduler.schedule(action)
                val waiting = withTimeout(90_000) {
                    while (repo.getResult(resultId)?.status != "needs_attention") {
                        val observed = runtime.findAutomationOccurrence(action.id, occurrence)
                            ?.let { runtime.observeAutomationExecution(it.id) }
                        val detail = observed?.get("latest_worker_observation") as? JsonObject
                        check(detail?.get("kind") != JsonPrimitive("disconnected")) { detail.toString() }
                        delay(250)
                    }
                    checkNotNull(repo.getResult(resultId))
                }
                assertNull(waiting.journalCompletedAt)
                val session = checkNotNull(waiting.journalSessionId)
                val original = checkNotNull(runtime.findAutomationOccurrence(action.id, occurrence))
                val noise = action.copy(id = "$prefix-other", name = "Other job")
                repo.create(noise)
                repeat(12) { n -> repo.recordResult(ExecutionResultEntity("other-$prefix-$n", noise.id, status = "answered", createdAt = scheduledAt + n + 1)) }
                ActivityScenario.launch(MainActivity::class.java).use {
                    val nav = hasTestTag("navigation.bottom")
                    fun waitFor(matcher: SemanticsMatcher) {
                        compose.waitUntil(25_000) { compose.onAllNodes(matcher).fetchSemanticsNodes().isNotEmpty() }
                    }
                    val skip = hasText(context.getString(R.string.onboarding_guide_action_later))
                    compose.waitUntil(20_000) { compose.onAllNodes(nav or skip).fetchSemanticsNodes().isNotEmpty() }
                    if (compose.onAllNodes(skip).fetchSemanticsNodes().isNotEmpty()) compose.onNode(skip).performClick()
                    val tab = context.getString(R.string.tab_activity)
                    val info = (hasText(tab) or hasContentDescription(tab)) and hasAnyAncestor(nav)
                    waitFor(info); compose.onNode(info).performClick()
                    val plans = hasContentDescription(context.getString(R.string.scheduled_title))
                    waitFor(plans); compose.onNode(plans).performClick()
                    val attention = hasText(context.getString(R.string.scheduled_filter_attention) + " (", substring = true) and hasClickAction()
                    val list = compose.onNode(hasScrollToIndexAction())
                    list.performScrollToNode(attention)
                    compose.onNode(attention).performScrollTo().performClick()
                    val result = hasTestTag("scheduled.result.${action.id}")
                    list.performScrollToNode(result)
                    compose.onNode(result).performScrollTo().performClick()
                    compose.onNodeWithText(context.getString(R.string.scheduled_result_completed_at)).assertDoesNotExist()
                    compose.onNodeWithTag("scheduled.result.openConversation").performClick()
                    val decision = if (daily) SemanticsMatcher("allow this permission once") {
                        it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("permission.allow-once.") == true
                    } else hasText(context.getString(R.string.common_deny))
                    compose.waitUntil(30_000) { compose.onAllNodes(decision or hasTestTag("chat-pending-permission")).fetchSemanticsNodes().isNotEmpty() }
                    if (compose.onAllNodes(decision).fetchSemanticsNodes().isEmpty()) compose.onNodeWithTag("chat-pending-permission").performClick()
                    waitFor(decision); compose.onNode(decision).performClick()
                    withTimeout(45_000) {
                        while (repo.getResult(resultId)?.journalCompletedAt == null || repo.getById(action.id)?.enabled != daily) delay(250)
                    }
                    if (daily) {
                        val work = WorkManager.getInstance(context)
                        val nextDay = target.plusDays(1).toInstant().toEpochMilli()
                        val name = "scheduled_occurrence_${action.id}:$nextDay"
                        val nextRuns = withTimeout(10_000) {
                            work.getWorkInfosForUniqueWorkFlow(name).first { rows -> rows.any { it.state == WorkInfo.State.ENQUEUED } }
                        }
                        assertEquals(1, nextRuns.count { !it.state.isFinished })
                    }
                }
                val terminal = checkNotNull(repo.getResult(resultId))
                val seal = runtime.observeAutomationExecution(original.id).getValue("terminal").jsonObject
                assertEquals(session, seal.getValue("identity").jsonObject.getValue("conversation_id").jsonPrimitive.content)
                assertEquals(terminal.journalCompletedAt, seal.getValue("completed_at_ms").jsonPrimitive.long)
                assertEquals(terminal.status, seal.getValue("result").jsonObject.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content)
                assertTrue("Unexpected terminal result ${terminal.status}", terminal.status in setOf("answered", "blocked", "incomplete"))
                val history = runtime.loadConversationProjectionV1(session)
                assertNull(history.activeRun)
                assertEquals(1, history.messages.count { it.role == com.nola.domain.projection.ConversationProjectionV1.Role.User })
                assertNull(terminal.failureReason)
                assertEquals(session, terminal.journalSessionId)
                assertEquals(original.id, runtime.findAutomationOccurrence(action.id, occurrence)?.id)
                assertEquals(1, repo.getResults(action.id).size)
                assertTrue(server.turns.get() >= 1)
                assertEquals(1, server.toolCalls.get())
            } finally {
                scheduler.cancel(action.id)
                repo.delete(action.id); repo.delete("$prefix-other")
                if (previousDefault == null) dependencies.preferences().clearDefaultModelSelection()
                else dependencies.preferences().saveDefaultModelSelection(previousDefault)
                runtime.deleteModelProfile(prefix, System.currentTimeMillis())
            }
        }
    }
}

private class ScheduledPermissionServer(private val authorize: Boolean) : AutoCloseable {
    private val socket = ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"))
    val url = "http://127.0.0.1:${socket.localPort}"
    val turns = AtomicInteger()
    val toolCalls = AtomicInteger()
    private val worker = thread(isDaemon = true) {
        try {
            while (!socket.isClosed) socket.accept().use { client ->
                val input = client.getInputStream().bufferedReader(Charsets.ISO_8859_1)
                val start = input.readLine() ?: return@use
                var length = 0
                while (true) {
                    val header = input.readLine()
                    if (header.isNullOrEmpty()) break
                    if (header.startsWith("Content-Length:", true)) length = header.substringAfter(':').trim().toInt()
                }
                val body = CharArray(length)
                var offset = 0
                while (offset < length) {
                    val n = input.read(body, offset, length - offset)
                    if (n <= 0) break
                    offset += n
                }
                val (type, response) = when {
                    start.startsWith("GET /source") -> "application/json" to """{"name":"scheduled fixture"}"""
                    start.startsWith("POST ") -> {
                        val first = turns.incrementAndGet() == 1
                        val review = String(body).contains("constraint-audit boundary")
                        if (first) toolCalls.incrementAndGet()
                        val delta = if (first) """{"tool_calls":[{"index":0,"id":"test-clipboard","type":"function","function":{"name":"device_clipboard_read","arguments":"{}"}}]}"""
                            else if (review) buildJsonObject {
                                put("content", if (authorize) """{"decision":"accept","outcome":"answered","effect_call_ids":[],"items":[],"violations":[]}"""
                                    else """{"decision":"blocked","outcome":"blocked","effect_call_ids":[],"items":[],"violations":[]}""")
                            }.toString()
                            else if (authorize) """{"content":"The clipboard check finished."}""" else """{"content":"The requested clipboard access was denied."}"""
                        val finish = if (first) "tool_calls" else "stop"
                        "text/event-stream" to "data: {\"choices\":[{\"delta\":$delta,\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"$finish\"}]}\n\ndata: [DONE]\n\n"
                    }
                    else -> "application/json" to "{}"
                }
                val output = client.getOutputStream()
                output.write("HTTP/1.1 200 OK\r\nContent-Type: $type\r\nContent-Length: ${response.toByteArray().size}\r\nConnection: close\r\n\r\n".toByteArray())
                output.write(response.toByteArray()); output.flush()
            }
        } catch (closed: SocketException) { if (!socket.isClosed) throw closed }
    }
    override fun close() { socket.close(); worker.join(2_000) }
}
