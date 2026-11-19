package com.nola.ui.screen.scheduled

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.ScheduledActionRepository
import com.nola.work.ScheduledActionScheduler
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.CancellationException
import javax.inject.Inject

data class ScheduledActionsUiState(
    val actions: List<ScheduledActionEntity> = emptyList(),
    val latestResultsByAction: Map<String, ExecutionResultEntity> = emptyMap(),
    val recentResults: List<ExecutionResultEntity> = emptyList(),
    val isLoading: Boolean = false,
    val errorMessage: String? = null,
    val busyActionIds: Set<String> = emptySet(),
    val savingActionId: String? = null,
    val saveErrorMessage: String? = null,
    val savePersistedBeforeFailure: Boolean = false,
)

@HiltViewModel
class ScheduledActionsViewModel internal constructor(
    private val repo: ScheduledActionRepository,
    private val schedule: (ScheduledActionEntity) -> Unit,
    private val cancel: (String) -> Unit,
) : ViewModel() {
    @Inject constructor(repo: ScheduledActionRepository, scheduler: ScheduledActionScheduler) :
        this(repo, scheduler::schedule, scheduler::cancel)

    private val _uiState = MutableStateFlow(ScheduledActionsUiState())
    val uiState: StateFlow<ScheduledActionsUiState> = _uiState.asStateFlow()

    private var observation: kotlinx.coroutines.Job? = null

    init { loadActions() }

    fun loadActions() {
        observation?.cancel()
        observation = viewModelScope.launch {
            _uiState.update { it.copy(isLoading = true, errorMessage = null) }
            try {
                kotlinx.coroutines.flow.combine(
                    repo.observeAll(), repo.observeLatestPerAction(), repo.observeRecentResults(limit = 10),
                ) { actions, latest, recent ->
                    Triple(actions, latest.associateBy { it.actionId }, recent)
                }.collectLatest { (actions, latest, recent) ->
                    _uiState.update { it.copy(actions = actions, latestResultsByAction = latest,
                        recentResults = recent, isLoading = false) }
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _uiState.update { it.copy(isLoading = false, errorMessage = error.localizedMessage.orEmpty()) }
            }
        }
    }

    fun beginEditing() {
        if (_uiState.value.savingActionId == null) {
            _uiState.update { it.copy(saveErrorMessage = null, savePersistedBeforeFailure = false) }
        }
    }

    fun save(action: ScheduledActionEntity, onSaved: () -> Unit = {}) {
        if (_uiState.value.savingActionId != null) return
        // Set synchronously before launching, so a second tap cannot enqueue another write.
        _uiState.update { it.copy(savingActionId = action.id, saveErrorMessage = null, savePersistedBeforeFailure = false) }
        viewModelScope.launch {
            var persisted = false
            try {
                repo.create(action)
                persisted = true
                schedule(action)
                _uiState.update { it.copy(savingActionId = null) }
                onSaved()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _uiState.update { it.copy(saveErrorMessage = error.localizedMessage.orEmpty(), savePersistedBeforeFailure = persisted) }
            } finally {
                _uiState.update { it.copy(savingActionId = null) }
            }
        }
    }

    fun delete(id: String) {
        mutate(id) {
            repo.delete(id)
            cancel(id)
        }
    }

    fun toggle(id: String, enabled: Boolean) {
        mutate(id) {
            repo.setEnabled(id, enabled)
            val action = repo.getById(id)
            if (enabled && action != null) {
                schedule(action.copy(enabled = true))
            } else {
                cancel(id)
            }
        }
    }

    private fun mutate(id: String, operation: suspend () -> Unit) {
        viewModelScope.launch {
            _uiState.update {
                it.copy(
                    errorMessage = null,
                    busyActionIds = it.busyActionIds + id
                )
            }
            runCatching { operation() }
                .onSuccess {
                    _uiState.update { state ->
                        state.copy(busyActionIds = state.busyActionIds - id)
                    }
                }
                .onFailure { error ->
                    if (error is CancellationException) throw error
                    _uiState.update { state ->
                        state.copy(
                            errorMessage = error.localizedMessage.orEmpty(),
                            busyActionIds = state.busyActionIds - id
                        )
                    }
                }
        }
    }
}
