package com.nola.ui.screen.onboarding

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.local.datastore.PreferenceDataStore
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import javax.inject.Inject

data class OnboardingUiState(
    val isCompleted: Boolean = false,
    val isCompleting: Boolean = false,
    val completionFailed: Boolean = false,
    val healthAvailability: AndroidHealthAvailability = AndroidHealthAvailability.Available,
    val healthPermissionState: AndroidHealthPermissionState = AndroidHealthPermissionState.Missing
)

@HiltViewModel
class OnboardingViewModel @Inject constructor(
    private val prefs: PreferenceDataStore,
    private val healthDataSource: AndroidHealthDataSource
) : ViewModel() {

    private val _uiState = MutableStateFlow(OnboardingUiState())
    val uiState: StateFlow<OnboardingUiState> = _uiState.asStateFlow()

    init {
        refreshHealthPermission()
    }

    fun refreshHealthPermission() {
        viewModelScope.launch {
            val result = withContext(Dispatchers.IO) {
                val availability = healthDataSource.availability()
                val permission = if (availability == AndroidHealthAvailability.Available && healthDataSource.hasReadPermission()) {
                    AndroidHealthPermissionState.Granted
                } else if (availability == AndroidHealthAvailability.Available) {
                    AndroidHealthPermissionState.Missing
                } else {
                    AndroidHealthPermissionState.Unavailable
                }
                availability to permission
            }
            _uiState.update { it.copy(healthAvailability = result.first, healthPermissionState = result.second) }
        }
    }

    fun completeOnboarding(runtimeChoice: String, selectedSources: Set<String>) {
        if (_uiState.value.isCompleting) return
        viewModelScope.launch {
            _uiState.update { it.copy(isCompleting = true, completionFailed = false) }
            runCatching {
                prefs.completeOnboarding(runtimeChoice, selectedSources)
            }.onSuccess {
                _uiState.update { it.copy(isCompleted = true, isCompleting = false) }
            }.onFailure {
                _uiState.update { it.copy(isCompleting = false, completionFailed = true) }
            }
        }
    }
}
