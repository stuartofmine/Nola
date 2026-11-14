// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.activity

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.data.local.AndroidApplicationRuntime
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*
import javax.inject.Inject

@Serializable
internal data class ActivityCursor(val created_at_ms: Long, val run_id: String)
@Serializable
internal data class ActivityQuery(val source: String?, val needs_response: Boolean,
    val query: String, val cursor: ActivityCursor?, val limit: Int)
@Serializable
internal data class ActivityEntry(val run_id: String, val conversation_id: String,
    val assistant_message_id: String, val source: String, val title: String,
    val created_at_ms: Long, val completed_at_ms: Long?, val status: String,
    val needs_response: Boolean, val is_current: Boolean)
@Serializable
internal data class ActivityPage(val entries: List<ActivityEntry>, val next_cursor: ActivityCursor?)
@Serializable
internal data class ActivityDetail(val entry: ActivityEntry, val explanation: String?,
    val source_conversation_id: String?, val review_in_source: Boolean, val result_text: String? = null, val result_has_more: Boolean = false,
    val execution_host_id: String?, val context_scope: String)
internal data class ActivityState(val entries: List<ActivityEntry> = emptyList(), val cursor: ActivityCursor? = null,
    val loading: Boolean = false, val failed: Boolean = false)
internal data class ActivityDetailState(val entry: ActivityEntry, val detail: ActivityDetail? = null,
    val loading: Boolean = true, val failed: Boolean = false)

/** Reads canonical Core projections. All decisions stay in the source conversation. */
@HiltViewModel
internal class ActivityViewModel internal constructor(
    private val executeQuery: suspend (JsonObject) -> JsonElement,
    private val ioDispatcher: CoroutineDispatcher,
    private val elapsedRealtime: () -> Long
) : ViewModel() {
    @Inject constructor(runtime: AndroidApplicationRuntime) : this(
        { runtime.executeQuery(it) }, Dispatchers.IO, { android.os.SystemClock.elapsedRealtime() }
    )
    private val json = Json { ignoreUnknownKeys = true }
    private val mutableState = MutableStateFlow(ActivityState())
    val state = mutableState.asStateFlow()
    private val mutableDetail = MutableStateFlow<ActivityDetailState?>(null)
    val detail = mutableDetail.asStateFlow()
    private var query = ActivityQuery(null, false, "", null, 40)
    private var listJob: Job? = null
    private var detailJob: Job? = null
    private var listGeneration = 0L
    private var detailGeneration = 0L
    private var lastLoadedAt = 0L

    fun resume(source: String?, needsResponse: Boolean, search: String) {
        val requested = ActivityQuery(source, needsResponse, search.trim(), null, 40)
        if (requested == query && (mutableState.value.loading ||
                (!mutableState.value.failed && lastLoadedAt > 0L &&
                    elapsedRealtime() - lastLoadedAt < 30_000L))) return
        refresh(source, needsResponse, search)
    }

    fun refresh(source: String?, needsResponse: Boolean, search: String) {
        val requested = ActivityQuery(source, needsResponse, search.trim(), null, 40)
        val sameQuery = requested == query
        query = requested
        listGeneration++
        listJob?.cancel()
        mutableState.value = if (sameQuery) mutableState.value.copy(loading = true, failed = false)
            else ActivityState(loading = true)
        load(query, append = false)
    }

    fun loadMore() {
        if (mutableState.value.loading) return
        val cursor = mutableState.value.cursor ?: return
        mutableState.value = mutableState.value.copy(loading = true, failed = false)
        load(query.copy(cursor = cursor), append = true)
    }

    private fun load(input: ActivityQuery, append: Boolean) {
        val generation = listGeneration
        listJob = viewModelScope.launch {
            try {
                val page = withContext(ioDispatcher) {
                    json.decodeFromJsonElement<ActivityPage>(executeQuery(buildJsonObject {
                        put("type", "list_activity"); put("input", json.encodeToJsonElement(input))
                    }))
                }
                if (generation != listGeneration) return@launch
                lastLoadedAt = elapsedRealtime()
                mutableState.value = ActivityState(
                    entries = if (append) (mutableState.value.entries + page.entries).distinctBy { it.run_id } else page.entries,
                    cursor = page.next_cursor)
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) {
                if (generation == listGeneration) mutableState.value = mutableState.value.copy(loading = false, failed = true)
            }
        }
    }

    fun open(entry: ActivityEntry) {
        val generation = ++detailGeneration
        detailJob?.cancel()
        mutableDetail.value = ActivityDetailState(entry)
        detailJob = viewModelScope.launch {
            try {
                val detail = withContext(ioDispatcher) {
                    val value = executeQuery(buildJsonObject {
                        put("type", "load_activity_detail"); put("conversation_id", entry.conversation_id)
                        put("run_id", entry.run_id); put("assistant_message_id", entry.assistant_message_id)
                    })
                    if (value == JsonNull) null else json.decodeFromJsonElement<ActivityDetail>(value)
                }
                if (generation != detailGeneration) return@launch
                require(detail == null || sameActivityOwner(entry, detail.entry))
                mutableDetail.value = ActivityDetailState(entry, detail, loading = false)
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) {
                if (generation == detailGeneration) mutableDetail.value = ActivityDetailState(entry, loading = false, failed = true)
            }
        }
    }

    fun closeDetail() { detailGeneration++; detailJob?.cancel(); mutableDetail.value = null }
}

internal fun sameActivityOwner(left: ActivityEntry, right: ActivityEntry): Boolean =
    left.run_id == right.run_id && left.conversation_id == right.conversation_id &&
        left.assistant_message_id == right.assistant_message_id
