package com.nola.ui.screen.home

import androidx.lifecycle.ViewModelStore
import com.nola.domain.model.CustomDataSourceConfig
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class DataSourcesViewModelTest {
    @Test
    fun clearingViewModelCancelsActiveWorkAndDiscardsQueuedReloads() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            val read = CompletableDeferred<Unit>()
            val save = CompletableDeferred<Unit>()
            var reads = 0
            var saves = 0
            var deletes = 0
            var callbacks = 0
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { _, previous ->
                    reads++
                    read.await()
                    previous.copy(isRefreshing = false)
                },
                save = { saves++; save.await() },
                delete = { deletes++ }, setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            val source = CustomDataSourceConfig(
                id = "source", name = "Source", location = "https://example.test/data"
            )
            viewModel.validateAndSave(source) { callbacks++ }
            viewModel.refresh()
            runCurrent()
            assertTrue(viewModel.uiState.value.isRefreshing)
            assertEquals("source", viewModel.uiState.value.validatingSourceId)
            owner.clear()
            runCurrent()
            assertFalse(viewModel.uiState.value.isRefreshing)
            assertEquals(null, viewModel.uiState.value.validatingSourceId)
            assertEquals(0, callbacks)
            // A stale UI callback must not create work or busy state after disposal.
            viewModel.refresh()
            viewModel.validateAndSave(source) { callbacks++ }
            viewModel.delete(source.id)
            read.complete(Unit)
            save.complete(Unit)
            runCurrent()
            assertEquals(1, reads)
            assertEquals(1, saves)
            assertEquals(0, deletes)
            assertEquals(0, callbacks)
            assertFalse(viewModel.uiState.value.isRefreshing)
            assertTrue(viewModel.uiState.value.busySourceIds.isEmpty())
            assertEquals(null, viewModel.uiState.value.validatingSourceId)
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }

    @Test
    fun failedRefreshReleasesBusyStateAndAllowsRetry() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            var reads = 0
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { _, previous ->
                    reads++
                    if (reads == 1) error("Snapshot unavailable")
                    previous.copy(isRefreshing = false, enabledDocumentCount = 7)
                }, save = {}, delete = {}, setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            assertFalse(viewModel.uiState.value.isRefreshing)
            assertEquals("Snapshot unavailable", viewModel.uiState.value.operationError)
            viewModel.clearOperationError()
            viewModel.refresh()
            runCurrent()
            assertEquals(2, reads)
            assertEquals(7, viewModel.uiState.value.enabledDocumentCount)
            assertFalse(viewModel.uiState.value.isRefreshing)
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }

    @Test
    fun failedMutationReloadsCommittedStateWithoutRetryingExternalReads() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            val loads = mutableListOf<Boolean>()
            var committed = false
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { refresh, previous ->
                    loads += refresh
                    previous.copy(isRefreshing = false, enabledDocumentCount = if (committed) 0 else 1)
                },
                save = {}, delete = { committed = true; error("Response lost") },
                setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            viewModel.delete("source")
            runCurrent()
            assertEquals(listOf(true, false), loads)
            assertEquals(0, viewModel.uiState.value.enabledDocumentCount)
            assertEquals("Response lost", viewModel.uiState.value.operationError)
            assertTrue(viewModel.uiState.value.busySourceIds.isEmpty())
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }

    @Test
    fun cancelledSaveClearsValidationWithoutReportingSuccess() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            var callbackCalled = false
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { _, previous -> previous.copy(isRefreshing = false) },
                save = { throw kotlinx.coroutines.CancellationException("Cancelled") },
                delete = {}, setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            viewModel.validateAndSave(CustomDataSourceConfig(
                id = "source", name = "Source", location = "https://example.test/data"
            )) { callbackCalled = true }
            runCurrent()
            assertEquals(null, viewModel.uiState.value.validatingSourceId)
            assertEquals(null, viewModel.uiState.value.operationError)
            assertFalse(callbackCalled)
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }

    @Test
    fun mutationDuringRefreshQueuesLocalReloadWithoutRestoringOldBusyState() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            val firstRead = CompletableDeferred<Unit>()
            val loads = mutableListOf<Boolean>()
            var deleted = false
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { refresh, previous ->
                    loads += refresh
                    if (loads.size == 1) firstRead.await()
                    previous.copy(isRefreshing = false, enabledDocumentCount = if (deleted) 0 else 1)
                },
                save = {}, delete = { deleted = true }, setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            viewModel.delete("source")
            runCurrent()
            assertTrue(deleted)
            assertTrue(viewModel.uiState.value.isRefreshing)
            assertTrue(viewModel.uiState.value.busySourceIds.isEmpty())
            firstRead.complete(Unit)
            runCurrent()
            assertEquals(listOf(true, false), loads)
            assertFalse(viewModel.uiState.value.isRefreshing)
            assertTrue(viewModel.uiState.value.busySourceIds.isEmpty())
            assertEquals(0, viewModel.uiState.value.enabledDocumentCount)
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }

    @Test
    fun refreshCannotClearInFlightValidationAndSaveOnlyReloadsLocalState() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        val owner = ViewModelStore()
        try {
            val firstRead = CompletableDeferred<Unit>()
            val save = CompletableDeferred<Unit>()
            val loads = mutableListOf<Boolean>()
            var completed: Boolean? = null
            val viewModel = DataSourcesViewModel(DataSourcesUiOperations(
                load = { refresh, previous ->
                    loads += refresh
                    if (loads.size == 1) firstRead.await()
                    previous.copy(isRefreshing = false)
                },
                save = { save.await() }, delete = {}, setEnabled = { _, _ -> }
            ), dispatcher)
            owner.put("sources", viewModel)
            runCurrent()
            viewModel.validateAndSave(CustomDataSourceConfig(
                id = "source", name = "Source", location = "https://example.test/data"
            )) { completed = it }
            runCurrent()
            firstRead.complete(Unit)
            runCurrent()
            assertEquals("source", viewModel.uiState.value.validatingSourceId)
            assertEquals(null, completed)
            save.complete(Unit)
            runCurrent()
            assertEquals(true, completed)
            assertEquals(null, viewModel.uiState.value.validatingSourceId)
            assertEquals(listOf(true, false), loads)
        } finally {
            owner.clear()
            Dispatchers.resetMain()
        }
    }
}
