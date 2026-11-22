package com.nola.ui.screen.notifications

import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.repository.NotificationRepository
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class NotificationCenterViewModelTest {

    @Test
    fun markReadFailureRollsBackOnlyTargetNotificationWithoutClobberingConcurrentRead() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        Dispatchers.setMain(dispatcher)
        try {
            val firstReadGate = CompletableDeferred<Unit>()
            val notifications = listOf(
                NotificationEntity(id = "a", title = "First", body = "A", source = "system", isRead = false),
                NotificationEntity(id = "b", title = "Second", body = "B", source = "system", isRead = false),
            )
            val repo = object : NotificationRepository {
                override suspend fun getNotifications(limit: Int): List<NotificationEntity> = notifications
                override suspend fun getUnread(): List<NotificationEntity> = notifications.filter { !it.isRead }
                override suspend fun unreadCount(): Int = notifications.count { !it.isRead }
                override suspend fun add(notification: NotificationEntity) {}
                override suspend fun markRead(id: String) {
                    if (id == "a") {
                        firstReadGate.await()
                        throw IllegalStateException("network failure")
                    }
                }
                override suspend fun markAllRead() {}
                override suspend fun syncFromBackend() {}
            }

            val viewModel = NotificationCenterViewModel(repo)
            runCurrent()
            assertEquals(2, viewModel.uiState.value.unreadCount)

            viewModel.markRead("a")
            viewModel.markRead("b")
            runCurrent()
            assertEquals(0, viewModel.uiState.value.unreadCount)
            assertTrue(viewModel.uiState.value.notifications.first { it.id == "b" }.isRead)

            firstReadGate.complete(Unit)
            runCurrent()

            val state = viewModel.uiState.value
            assertTrue(state.hasLoadError)
            assertFalse(state.notifications.first { it.id == "a" }.isRead)
            assertTrue(state.notifications.first { it.id == "b" }.isRead)
            assertEquals(1, state.unreadCount)
        } finally {
            Dispatchers.resetMain()
        }
    }
}
