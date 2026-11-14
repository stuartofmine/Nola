package com.nola.ui.screen.activity

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ActivityViewModelTest {
    private fun entry(id: String = "run", conversation: String = "conversation") = ActivityEntry(
        id, conversation, "assistant", "agent", "Task", 1, null, "completed", false, true)

    @Test fun refreshFailureRetainsSameQuerySnapshotAndResumeAvoidsUnnecessaryRead() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            var reads = 0
            var fail = false
            var now = 1L
            val model = ActivityViewModel({
                reads++
                if (fail) error("offline")
                Json.encodeToJsonElement(ActivityPage(listOf(entry()), ActivityCursor(1, "run")))
            }, dispatcher) { now }
            model.resume(null, false, "")
            advanceUntilIdle()
            assertEquals(1, reads)
            model.resume(null, false, "")
            advanceUntilIdle()
            assertEquals(1, reads)
            fail = true
            model.refresh(null, false, "")
            assertEquals(listOf(entry()), model.state.value.entries)
            advanceUntilIdle()
            assertTrue(model.state.value.failed)
            assertEquals(listOf(entry()), model.state.value.entries)
            // A different query must not expose the old rows or pagination.
            model.refresh("automation", false, "new")
            assertTrue(model.state.value.entries.isEmpty())
            assertNull(model.state.value.cursor)
            advanceUntilIdle()
            fail = false
            now += 30_001L
            model.resume("automation", false, "new")
            advanceUntilIdle()
            assertFalse(model.state.value.failed)
        } finally { Dispatchers.resetMain() }
    }

    @Test fun detailRejectsOwnerMismatchInsteadOfOpeningAnotherConversationsResult() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            val model = ActivityViewModel({
                Json.encodeToJsonElement(ActivityDetail(entry(conversation = "other"), null,
                    "other", false, "Private result", false, null, "conversation"))
            }, dispatcher) { 1L }
            model.open(entry())
            advanceUntilIdle()
            assertTrue(model.detail.value!!.failed)
            assertNull(model.detail.value!!.detail)
            assertEquals("conversation", model.detail.value!!.entry.conversation_id)
        } finally { Dispatchers.resetMain() }
    }

    @Test fun changingFiltersInvalidatesPaginationAndLateOldReads() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            val old = CompletableDeferred<JsonElement>()
            val calls = mutableListOf<ActivityQuery>()
            val model = ActivityViewModel({ request ->
                val input = Json.decodeFromJsonElement<ActivityQuery>(request.getValue("input"))
                calls += input
                if (input.query == "old") old.await()
                else Json.encodeToJsonElement(ActivityPage(listOf(entry(id = "new")), null))
            }, dispatcher) { 1L }
            model.refresh(null, false, "old")
            runCurrent()
            model.refresh("automation", true, "new")
            model.loadMore()
            advanceUntilIdle()
            old.complete(Json.encodeToJsonElement(ActivityPage(listOf(entry(id = "old")), ActivityCursor(1, "old"))))
            advanceUntilIdle()
            assertEquals(listOf("new"), model.state.value.entries.map { it.run_id })
            assertEquals(2, calls.size)
            assertTrue(calls.all { it.cursor == null })
            assertEquals("automation", calls.last().source)
            assertTrue(calls.last().needs_response)
        } finally { Dispatchers.resetMain() }
    }
}
