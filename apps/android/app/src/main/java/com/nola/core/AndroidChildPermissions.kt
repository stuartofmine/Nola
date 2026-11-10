package com.nola.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Only routes a displayed decision to its live, exact child request. Core grants authority. */
internal object AndroidChildPermissions {
    data class Pending(val parentConversationId: String, val childConversationId: String,
        val request: AndroidAgentPermissionRequest)
    private data class Entry(val pending: Pending, val decision: CompletableDeferred<AndroidAgentPermissionDecision>)
    private val entries = linkedMapOf<String, Entry>()
    private val mutablePending = MutableStateFlow<List<Pending>>(emptyList())
    val pending = mutablePending.asStateFlow()

    fun owns(requestId: String): Boolean = synchronized(entries) { entries.containsKey(requestId) }

    suspend fun await(parent: String, child: String, request: AndroidAgentPermissionRequest): AndroidAgentPermissionDecision {
        val entry = Entry(Pending(parent, child, request), CompletableDeferred())
        synchronized(entries) {
            check(entries.putIfAbsent(request.requestId, entry) == null) { "Duplicate child permission owner" }
            mutablePending.value = entries.values.map { it.pending }
        }
        try { return entry.decision.await() }
        finally {
            synchronized(entries) {
                if (entries[request.requestId] === entry) entries.remove(request.requestId)
                mutablePending.value = entries.values.map { it.pending }
            }
        }
    }

    fun resolve(decision: AndroidAgentPermissionDecision): Boolean = synchronized(entries) {
        entries[decision.requestId]?.decision?.complete(decision) ?: false
    }
}
