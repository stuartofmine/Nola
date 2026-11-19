package com.nola.ui.screen.onboarding

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.datastore.PreferenceDataStore
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import javax.inject.Inject

data class OnboardingUiState(
    val isCompleting: Boolean = false,
    val completionFailed: Boolean = false,
)

@HiltViewModel
class OnboardingViewModel @Inject constructor(private val prefs: PreferenceDataStore) : ViewModel() {
    private val state = MutableStateFlow(OnboardingUiState())
    val uiState = state.asStateFlow()

    fun completeOnboarding(): Boolean {
        if (state.value.isCompleting) return false
        state.value = OnboardingUiState(isCompleting = true)
        viewModelScope.launch {
            try {
                prefs.setOnboardingCompleted()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                state.value = OnboardingUiState(completionFailed = true)
            }
        }
        return true
    }
}
