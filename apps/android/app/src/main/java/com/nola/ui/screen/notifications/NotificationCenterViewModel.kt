package com.nola.ui.screen.notifications

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.db.entity.NotificationEntity
import com.nola.data.repository.NotificationRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import javax.inject.Inject

data class NotificationCenterUiState(
    val notifications: List<NotificationEntity> = emptyList(),
    val unreadCount: Int = 0,
    val isLoading: Boolean = true,
    val isMarkingAllRead: Boolean = false,
    val hasLoadError: Boolean = false,
    val hasSyncWarning: Boolean = false
)

@HiltViewModel
class NotificationCenterViewModel @Inject constructor(
    private val repo: NotificationRepository
) : ViewModel() {

    private val _uiState = MutableStateFlow(NotificationCenterUiState())
    val uiState: StateFlow<NotificationCenterUiState> = _uiState.asStateFlow()

    init {
        loadNotifications()
    }

    fun loadNotifications() {
        viewModelScope.launch {
            refreshNotifications(showLoading = true)
        }
    }

    fun markRead(id: String) {
        val previousItem = _uiState.value.notifications.firstOrNull { it.id == id }
        val wasUnread = previousItem?.isRead == false
        val previousReadAt = previousItem?.readAt
        _uiState.update { state ->
            val itemUnread = state.notifications.firstOrNull { it.id == id }?.isRead == false
            state.copy(
                notifications = state.notifications.map { notification ->
                    if (notification.id == id) notification.copy(isRead = true, readAt = System.currentTimeMillis()) else notification
                },
                unreadCount = if (itemUnread) (state.unreadCount - 1).coerceAtLeast(0) else state.unreadCount
            )
        }
        viewModelScope.launch {
            try {
                repo.markRead(id)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                _uiState.update { state ->
                    val currentItem = state.notifications.firstOrNull { it.id == id }
                    val shouldRestoreUnread = wasUnread && currentItem?.isRead == true
                    state.copy(
                        notifications = if (wasUnread) {
                            state.notifications.map { notification ->
                                if (notification.id == id) {
                                    notification.copy(isRead = false, readAt = previousReadAt)
                                } else {
                                    notification
                                }
                            }
                        } else {
                            state.notifications
                        },
                        unreadCount = if (shouldRestoreUnread) state.unreadCount + 1 else state.unreadCount,
                        hasLoadError = true
                    )
                }
            }
        }
    }

    fun markAllRead() {
        viewModelScope.launch {
            _uiState.update { it.copy(isMarkingAllRead = true) }
            val result = runCatching { repo.markAllRead() }
            result.exceptionOrNull()?.let { if (it is kotlinx.coroutines.CancellationException) throw it }
            if (result.isSuccess) {
                _uiState.update { state ->
                    state.copy(
                        notifications = state.notifications.map { it.copy(isRead = true, readAt = System.currentTimeMillis()) },
                        unreadCount = 0,
                        isMarkingAllRead = false,
                        hasLoadError = false
                    )
                }
            } else {
                _uiState.update { it.copy(isMarkingAllRead = false, hasLoadError = true) }
            }
        }
    }

    private suspend fun refreshNotifications(showLoading: Boolean) {
        if (showLoading) _uiState.update { it.copy(isLoading = true) }
        val syncResult = runCatching { repo.syncFromBackend() }
        syncResult.exceptionOrNull()?.let { if (it is kotlinx.coroutines.CancellationException) throw it }
        val localResult = runCatching {
            repo.getNotifications(limit = 100) to repo.unreadCount()
        }
        localResult.exceptionOrNull()?.let { if (it is kotlinx.coroutines.CancellationException) throw it }
        localResult.onSuccess { (notifications, unread) ->
            _uiState.update {
                it.copy(
                    notifications = notifications,
                    unreadCount = unread,
                    isLoading = false,
                    hasLoadError = false,
                    hasSyncWarning = syncResult.isFailure
                )
            }
        }.onFailure {
            _uiState.update {
                it.copy(isLoading = false, hasLoadError = true, hasSyncWarning = syncResult.isFailure)
            }
        }
    }
}
