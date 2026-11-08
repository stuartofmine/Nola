package com.nola.ui.generative

import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableLongStateOf
import com.nola.data.local.UiSnapshot
import com.nola.data.local.UiSurfacePhase
import com.nola.data.local.UiSubmission
import com.nola.data.local.UiSurfaceIdentity
import kotlinx.serialization.json.JsonElement

/** Process-lifetime UI editing buffers, isolated by the complete Core identity.
 * No committed data is owned here, and form values are never written to disk. */
internal class UiDraftState {
    val drafts = mutableStateMapOf<Pair<String, String>, JsonElement>()
    val generation = mutableLongStateOf(1)
    val revision = mutableStateOf<Long?>(null)
    val unresolved = mutableStateOf<UiSubmission?>(null)

    /** Explicit user confirmation retains values; Core still validates the draft. */
    fun revalidate(snapshot: UiSnapshot, blocked: Boolean): Boolean {
        if (blocked || unresolved.value != null || snapshot.phase != UiSurfacePhase.Ready) return false
        if (drafts.keys.any { (node, binding) -> snapshot.nodes[node]?.bindings?.containsKey(binding) != true }) return false
        revision.value = snapshot.revision
        generation.longValue += 1
        return true
    }
}

internal object UiDraftSession {
    private val entries = mutableMapOf<UiSurfaceIdentity, UiDraftState>()
    fun forSurface(identity: UiSurfaceIdentity): UiDraftState = entries.getOrPut(identity) { UiDraftState() }
    fun releaseIfEmpty(identity: UiSurfaceIdentity, state: UiDraftState) {
        if (state.drafts.isEmpty() && state.unresolved.value == null && entries[identity] === state) entries.remove(identity)
    }
}
