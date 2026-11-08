package com.nola.ui.screen.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.health.AndroidHealthSummary
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.repository.AndroidDataSourceRepository
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceUsageRecord
import dagger.hilt.android.lifecycle.HiltViewModel
import java.time.Instant
import javax.inject.Inject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class DataSourcesUiState(
    val isRefreshing: Boolean = false,
    val health: AndroidHealthSummary? = null,
    val enabledDocumentCount: Int = 0,
    val documentCharacterCount: Int = 0,
    val enabledToolCount: Int = 0,
    val enabledSkillCount: Int = 0,
    val lastSuccessfulReadAt: Instant? = null,
    val customSources: List<CustomDataSourceConfig> = emptyList(),
    val recentUsage: List<DataSourceUsageRecord> = emptyList(),
    val validatingSourceId: String? = null,
    val busySourceIds: Set<String> = emptySet(),
    val operationError: String? = null
)

@HiltViewModel
class DataSourcesViewModel internal constructor(
    private val operations: DataSourcesUiOperations,
    private val ioDispatcher: CoroutineDispatcher
) : ViewModel() {
    @Inject
    constructor(
        healthDataSource: AndroidHealthDataSource,
        memoryProfileDataStore: MemoryProfileDataStore,
        dataSourceRepository: AndroidDataSourceRepository
    ) : this(
        DataSourcesUiOperations(
            load = { refreshSources, previous ->
                loadDataSourcesSnapshot(healthDataSource, memoryProfileDataStore,
                    dataSourceRepository, refreshSources, previous)
            },
            save = { dataSourceRepository.validateAndUpsert(it); Unit },
            delete = { dataSourceRepository.delete(it) },
            setEnabled = { id, enabled -> dataSourceRepository.setEnabled(id, enabled) }
        ),
        Dispatchers.IO
    )

    private val _uiState = MutableStateFlow(DataSourcesUiState())
    val uiState = _uiState.asStateFlow()
    private var reloadPending = false
    private var sourceReadPending = false

    init {
        refresh()
    }

    fun refresh() {
        loadData(refreshSources = true)
    }

    private fun loadData(refreshSources: Boolean) {
        if (!viewModelScope.isActive) return
        if (_uiState.value.isRefreshing) {
            reloadPending = true
            sourceReadPending = sourceReadPending || refreshSources
            return
        }
        _uiState.value = _uiState.value.copy(isRefreshing = true)
        viewModelScope.launch {
            val previous = _uiState.value
            try {
                val outcome = withContext(ioDispatcher) {
                    sourceOperation { operations.load(refreshSources, previous) }
                }
                val current = _uiState.value
                _uiState.value = outcome.getOrElse { error ->
                    current.copy(operationError = error.localizedMessage ?: error.javaClass.simpleName)
                }.copy(
                    isRefreshing = false,
                    validatingSourceId = current.validatingSourceId,
                    busySourceIds = current.busySourceIds,
                    operationError = outcome.exceptionOrNull()?.let { it.localizedMessage ?: it.javaClass.simpleName }
                        ?: outcome.getOrNull()?.operationError ?: current.operationError
                )
            } finally {
                _uiState.value = _uiState.value.copy(isRefreshing = false)
                if (reloadPending) {
                    val readSources = sourceReadPending
                    reloadPending = false
                    sourceReadPending = false
                    if (viewModelScope.isActive) loadData(refreshSources = readSources)
                }
            }
        }
    }

    fun validateAndSave(source: CustomDataSourceConfig, onComplete: (Boolean) -> Unit = {}) {
        if (!viewModelScope.isActive) return
        if (_uiState.value.validatingSourceId != null) return
        _uiState.value = _uiState.value.copy(validatingSourceId = source.id, operationError = null)
        viewModelScope.launch {
            val result = try {
                withContext(ioDispatcher) { sourceOperation { operations.save(source) } }
            } finally {
                _uiState.value = _uiState.value.copy(validatingSourceId = null)
            }
            _uiState.value = _uiState.value.copy(
                validatingSourceId = null,
                operationError = result.exceptionOrNull()?.localizedMessage
            )
            loadData(refreshSources = false)
            onComplete(result.isSuccess)
        }
    }

    fun delete(sourceId: String) {
        mutate(sourceId) { operations.delete(sourceId) }
    }

    fun setEnabled(sourceId: String, enabled: Boolean) {
        mutate(sourceId) { operations.setEnabled(sourceId, enabled) }
    }

    private fun mutate(sourceId: String, operation: suspend () -> Unit) {
        if (!viewModelScope.isActive) return
        if (sourceId in _uiState.value.busySourceIds) return
        _uiState.value = _uiState.value.copy(
            busySourceIds = _uiState.value.busySourceIds + sourceId,
            operationError = null
        )
        viewModelScope.launch {
            val result = try {
                withContext(ioDispatcher) { sourceOperation { operation() } }
            } finally {
                _uiState.value = _uiState.value.copy(busySourceIds = _uiState.value.busySourceIds - sourceId)
            }
            _uiState.value = _uiState.value.copy(
                busySourceIds = _uiState.value.busySourceIds - sourceId,
                operationError = result.exceptionOrNull()?.localizedMessage
            )
            loadData(refreshSources = false)
        }
    }

    fun clearOperationError() {
        _uiState.value = _uiState.value.copy(operationError = null)
    }
}

internal data class DataSourcesUiOperations(
    val load: suspend (Boolean, DataSourcesUiState) -> DataSourcesUiState,
    val save: suspend (CustomDataSourceConfig) -> Unit,
    val delete: suspend (String) -> Unit,
    val setEnabled: suspend (String, Boolean) -> Unit
)

private suspend fun loadDataSourcesSnapshot(
    healthDataSource: AndroidHealthDataSource,
    memoryProfileDataStore: MemoryProfileDataStore,
    dataSourceRepository: AndroidDataSourceRepository,
    refreshSources: Boolean,
    previous: DataSourcesUiState
): DataSourcesUiState {
                val refreshResult = sourceOperation {
                    if (refreshSources) dataSourceRepository.refreshAll()
                }
                val profileResult = sourceOperation { memoryProfileDataStore.getSnapshot() }
                val profile = profileResult.getOrNull()
                val health = if (!refreshSources) previous.health else sourceOperation {
                    healthDataSource.readSummary(days = 7)
                }.getOrElse { error ->
                    val availability = healthDataSource.availability()
                    AndroidHealthSummary(
                        availability = availability,
                        permissionState = if (availability == AndroidHealthAvailability.Available) {
                            AndroidHealthPermissionState.Missing
                        } else {
                            AndroidHealthPermissionState.Unavailable
                        },
                        error = error.localizedMessage ?: error.javaClass.simpleName
                    )
                }
                return DataSourcesUiState(
                    isRefreshing = false,
                    health = health,
                    enabledDocumentCount = profile?.uploadedDocuments?.count { it.enabled && it.content.isNotBlank() }
                        ?: previous.enabledDocumentCount,
                    documentCharacterCount = profile?.uploadedDocuments
                        ?.filter { it.enabled }
                        ?.sumOf { it.content.length }
                        ?: previous.documentCharacterCount,
                    enabledToolCount = profile?.mcpServers?.count { it.enabled }
                        ?: previous.enabledToolCount,
                    enabledSkillCount = profile?.uploadedSkills?.count { it.enabled && it.content.isNotBlank() }
                        ?: previous.enabledSkillCount,
                    lastSuccessfulReadAt = health?.updatedAt
                        ?.takeIf { health.permissionState == AndroidHealthPermissionState.Granted && health.error == null }
                        ?: profile?.customDataSources?.mapNotNull { it.lastSuccessfulReadAt }?.maxOrNull()?.let(Instant::ofEpochMilli)
                        ?: previous.lastSuccessfulReadAt,
                    customSources = profile?.customDataSources ?: previous.customSources,
                    recentUsage = profile?.dataSourceUsageRecords?.sortedByDescending { it.usedAt }?.take(20)
                        ?: previous.recentUsage,
                    validatingSourceId = previous.validatingSourceId,
                    busySourceIds = previous.busySourceIds,
                    operationError = (refreshResult.exceptionOrNull() ?: profileResult.exceptionOrNull())?.localizedMessage
                )
}

private inline fun <T> sourceOperation(block: () -> T): Result<T> = try {
    Result.success(block())
} catch (error: CancellationException) {
    throw error
} catch (error: Exception) {
    Result.failure(error)
}
