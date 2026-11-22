// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.notifications

import android.graphics.Bitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.repository.NotificationRepository
import com.nola.ui.theme.NolaTheme
import com.nola.ui.screen.chat.AndroidExpertNativeTestDependencies
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import dagger.hilt.android.EntryPointAccessors
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import java.io.File
import org.junit.Rule
import org.junit.Test

class NotificationCenterInteractionTest {
    @get:Rule val compose = createComposeRule()

    @Test fun localOnlyNotificationHistoryLoadsWithoutAnAccount() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val dependencies = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java)
        val repository = dependencies.notifications()
        val empty = runBlocking {
            check(dependencies.preferences().backendAccessToken.first().isBlank())
            repository.getNotifications(1).isEmpty()
        }
        val model = NotificationCenterViewModel(repository)
        compose.setContent { NolaTheme { NotificationCenterScreen(viewModel = model) } }
        compose.waitUntil(10_000) { !model.uiState.value.isLoading }
        check(!model.uiState.value.hasSyncWarning && !model.uiState.value.hasLoadError)
        if (empty) compose.onNodeWithText(context.getString(R.string.notifications_no_notifications)).assertIsDisplayed()
    }

    @Test fun emptyCacheOfflineRetriesAndSearchKeepsUnreadFilter() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        var attempts = 0
        val repo = object : NotificationRepository {
            val rows = listOf(
                NotificationEntity("new", "Review ready"),
                NotificationEntity("old", "Archived review", isRead = true)
            )
            override suspend fun syncFromBackend() { if (++attempts == 1) error("Offline fixture") }
            override suspend fun getNotifications(limit: Int) = if (attempts > 1) rows else emptyList()
            override suspend fun getUnread() = getNotifications(100).filter { !it.isRead }
            override suspend fun unreadCount() = getUnread().size
            override suspend fun add(notification: NotificationEntity) = Unit
            override suspend fun markRead(id: String) = Unit
            override suspend fun markAllRead() = Unit
        }
        val model = NotificationCenterViewModel(repo)
        compose.setContent { NolaTheme { NotificationCenterScreen(viewModel = model) } }
        compose.onNodeWithText(context.getString(R.string.notifications_load_failed)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.notifications_no_notifications)).assertDoesNotExist()
        val automation = instrumentation.uiAutomation
        compose.waitUntil(10_000) { automation.rootInActiveWindow?.packageName == context.packageName }
        automation.waitForIdle(350, 5_000)
        val frame = automation.takeScreenshot()
        check(automation.rootInActiveWindow?.packageName == context.packageName)
        File(context.cacheDir, "notification-offline.png").outputStream().use { stream ->
            check(frame.compress(Bitmap.CompressFormat.PNG, 100, stream))
        }
        frame.recycle()
        compose.onNodeWithText(context.getString(R.string.notifications_retry)).performClick()
        compose.onNodeWithText("Archived review").assertIsDisplayed()
        val unread = compose.onNodeWithText(context.getString(R.string.notifications_filter_unread_v2) + " (1)")
        unread.performClick()
        compose.onNode(hasSetTextAction()).performTextInput("absent")
        compose.onNode(hasSetTextAction()).performImeAction()
        compose.onNodeWithText(context.getString(R.string.notifications_clear_search)).performClick()
        unread.assertIsSelected()
        compose.onNodeWithText("Review ready").assertIsDisplayed()
        compose.onNodeWithText("Archived review").assertDoesNotExist()
    }
}
