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
    val isLoading: Boolean = false,
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
        val previous = _uiState.value
        _uiState.update { state ->
            val wasUnread = state.notifications.firstOrNull { it.id == id }?.isRead == false
            state.copy(
                notifications = state.notifications.map { notification ->
                    if (notification.id == id) notification.copy(isRead = true, readAt = System.currentTimeMillis()) else notification
                },
                unreadCount = if (wasUnread) (state.unreadCount - 1).coerceAtLeast(0) else state.unreadCount
            )
        }
        viewModelScope.launch {
            runCatching { repo.markRead(id) }
                .onFailure {
                    _uiState.value = previous.copy(hasLoadError = true)
                }
        }
    }

    fun markAllRead() {
        viewModelScope.launch {
            _uiState.update { it.copy(isMarkingAllRead = true) }
            val result = runCatching { repo.markAllRead() }
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
        val localResult = runCatching {
            repo.getNotifications(limit = 100) to repo.unreadCount()
        }
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
