// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import com.nola.data.local.TodayFactsSnapshot
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import javax.inject.Inject

internal data class TodayPendingResponsesState(
    val snapshot: TodayFactsSnapshot? = null,
    val loading: Boolean = true,
    val failed: Boolean = false,
)

/** Transient presentation only: no permission decisions or business state are cached. */
@HiltViewModel
internal class TodayPendingResponsesViewModel @Inject constructor(
    private val reader: TodayPendingResponseReader,
) : ViewModel() {
    private val mutableState = MutableStateFlow(TodayPendingResponsesState())
    val state = mutableState.asStateFlow()
    private var refreshJob: Job? = null

    fun refresh() {
        if (refreshJob?.isActive == true) return
        refreshJob = viewModelScope.launch {
            mutableState.value = mutableState.value.copy(loading = true, failed = false)
            try {
                val snapshot = withContext(Dispatchers.IO) { reader.read() }
                mutableState.value = TodayPendingResponsesState(snapshot = snapshot, loading = false)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                mutableState.value = mutableState.value.copy(loading = false, failed = true)
            }
        }
    }
}
