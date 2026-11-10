package com.nola.ui.screen.scheduled

import androidx.lifecycle.ViewModelStore
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.ScheduledActionRepository
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.test.*
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ScheduledActionSaveTest {
    private val action = ScheduledActionEntity("draft-id", "local", "Task", "https://example.test", "url",
        scheduleFrequency = "once", scheduledTimeIso = "2026-10-09T09:00:00Z")

    @Test fun savingBlocksDuplicateSubmissionAndClosesOnlyAfterCompletion() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val owner = ViewModelStore()
        try {
            val pending = CompletableDeferred<Unit>()
            val repo = FakeRepo { pending.await() }
            var callbacks = 0
            var schedules = 0
            val model = ScheduledActionsViewModel(repo, { schedules++ }, {})
            owner.put("editor", model)
            model.save(action) { callbacks++ }
            model.save(action) { callbacks++ }
            assertEquals(action.id, model.uiState.value.savingActionId)
            runCurrent()
            assertEquals(1, repo.writes)
            assertEquals(0, callbacks)
            pending.complete(Unit)
            runCurrent()
            assertEquals(1, schedules)
            assertEquals(1, callbacks)
            assertNull(model.uiState.value.savingActionId)
        } finally { owner.clear(); Dispatchers.resetMain() }
    }

    @Test fun failedPersistenceKeepsDraftOpenAndRetryUsesSameId() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val owner = ViewModelStore()
        try {
            var fail = true
            val repo = FakeRepo { if (fail) error("Disk unavailable") }
            var callbacks = 0
            val model = ScheduledActionsViewModel(repo, {}, {})
            owner.put("editor", model)
            model.save(action) { callbacks++ }
            runCurrent()
            assertEquals(0, callbacks)
            assertEquals("Disk unavailable", model.uiState.value.saveErrorMessage)
            assertFalse(model.uiState.value.savePersistedBeforeFailure)
            fail = false
            model.save(action) { callbacks++ }
            runCurrent()
            assertEquals(1, callbacks)
            assertNull(model.uiState.value.saveErrorMessage)
            assertEquals(listOf(action.id, action.id), repo.ids)
        } finally { owner.clear(); Dispatchers.resetMain() }
    }

    @Test fun schedulingFailureIsNotMisreportedAsNoPersistence() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val owner = ViewModelStore()
        try {
            val model = ScheduledActionsViewModel(FakeRepo {}, { error("Scheduling unavailable") }, {})
            owner.put("editor", model)
            var callbacks = 0
            model.save(action) { callbacks++ }
            runCurrent()
            assertEquals(0, callbacks)
            assertTrue(model.uiState.value.savePersistedBeforeFailure)
            assertEquals("Scheduling unavailable", model.uiState.value.saveErrorMessage)
            assertNull(model.uiState.value.savingActionId)
        } finally { owner.clear(); Dispatchers.resetMain() }
    }

    @Test fun cancellationDoesNotCloseEditorOrInventSaveFailure() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val owner = ViewModelStore()
        try {
            val pending = CompletableDeferred<Unit>()
            val model = ScheduledActionsViewModel(FakeRepo { pending.await() }, {}, {})
            owner.put("editor", model)
            var callbacks = 0
            model.save(action) { callbacks++ }
            runCurrent()
            owner.clear()
            runCurrent()
            assertEquals(0, callbacks)
            assertNull(model.uiState.value.saveErrorMessage)
            assertNull(model.uiState.value.savingActionId)
        } finally { owner.clear(); Dispatchers.resetMain() }
    }

    private class FakeRepo(private val write: suspend () -> Unit) : ScheduledActionRepository {
        var writes = 0
        val ids = mutableListOf<String>()
        override suspend fun create(action: ScheduledActionEntity) { writes++; ids += action.id; write() }
        override suspend fun getAll() = emptyList<ScheduledActionEntity>()
        override suspend fun getById(id: String): ScheduledActionEntity? = null
        override suspend fun delete(id: String) = Unit
        override suspend fun setEnabled(id: String, enabled: Boolean) = Unit
        override suspend fun getResults(actionId: String) = emptyList<ExecutionResultEntity>()
        override suspend fun getRecentResults(limit: Int) = emptyList<ExecutionResultEntity>()
        override fun observeRecentResults(limit: Int) = emptyFlow<List<ExecutionResultEntity>>()
        override suspend fun recordResult(result: ExecutionResultEntity) = Unit
    }
}
