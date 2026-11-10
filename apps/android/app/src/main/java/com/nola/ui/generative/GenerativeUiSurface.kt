package com.nola.ui.generative

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import com.nola.R
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import com.nola.data.local.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonElement
import java.util.UUID

/** Stable semantic ownership for accessibility tooling and real UI acceptance. */
val UiSurfaceIdentitySemantics = SemanticsPropertyKey<UiSurfaceIdentity>("UiSurfaceIdentity")

sealed interface UiSubmissionOutcome {
    data class Committed(val receipt: UiInteractionReceipt) : UiSubmissionOutcome
    data class Rejected(val message: String) : UiSubmissionOutcome
    data class Unknown(val message: String) : UiSubmissionOutcome
}

/** Mount under key(snapshot.identity). Revision changes retain focus and drafts. */
@Composable
fun GenerativeUiSurface(
    snapshot: UiSnapshot,
    submit: suspend (UiSubmission) -> UiSubmissionOutcome,
    accepted: (UiInteractionReceipt) -> Unit,
    restore: suspend (UiSurfaceIdentity) -> UiSubmission? = { null },
    preview: (suspend (UiPreviewInput) -> UiPreview)? = null,
) {
    key(snapshot.identity) { SurfaceContent(snapshot, submit, accepted, restore, preview) }
}

@Composable
private fun SurfaceContent(
    snapshot: UiSnapshot,
    submit: suspend (UiSubmission) -> UiSubmissionOutcome,
    accepted: (UiInteractionReceipt) -> Unit,
    restore: suspend (UiSurfaceIdentity) -> UiSubmission?,
    preview: (suspend (UiPreviewInput) -> UiPreview)?,
) {
    val context = LocalContext.current
    var restoreAttempt by remember { mutableIntStateOf(0) }
    var restoreFailed by remember { mutableStateOf(false) }
    val composing = remember { mutableStateMapOf<String, Boolean>() }
    val maximumDialogHeight = (LocalConfiguration.current.screenHeightDp * 0.85f).dp
    val editing = remember(snapshot.identity) {
        if (snapshot.phase != UiSurfacePhase.ReadOnly && snapshot.phase != UiSurfacePhase.Deleted) UiDraftSession.forSurface(snapshot.identity) else UiDraftState()
    }
    val drafts = editing.drafts
    var draftRevision by editing.revision
    var pending by remember { mutableStateOf(false) }
    var restoring by remember { mutableStateOf(snapshot.phase == UiSurfacePhase.Ready || snapshot.phase == UiSurfacePhase.Pending) }
    var unresolved by editing.unresolved
    var failure by remember { mutableStateOf<String?>(null) }
    var modal by remember { mutableStateOf<String?>(null) }
    DisposableEffect(snapshot.identity) {
        onDispose { UiDraftSession.releaseIfEmpty(snapshot.identity, editing) }
    }
    val scope = rememberCoroutineScope()
    val latest by rememberUpdatedState(snapshot)
    val currentRestore by rememberUpdatedState(restore)
    val currentSubmit by rememberUpdatedState(submit)
    val currentAccepted by rememberUpdatedState(accepted)
    val currentPreview by rememberUpdatedState(preview)
    var draftGeneration by editing.generation
    var candidate by remember { mutableStateOf<UiPreview?>(null) }
    var previewFailure by remember { mutableStateOf<String?>(null) }
    var previewRetry by remember { mutableIntStateOf(0) }
    val hasDraft = drafts.isNotEmpty()
    val previewAccepted = candidate?.takeIf { hasDraft && it.snapshot.catalog_digest == snapshot.catalog_digest && it.snapshot.catalog_id == snapshot.catalog_id && it.identity == snapshot.identity && it.base_revision == snapshot.revision &&
        it.draft_generation == draftGeneration && composing.isEmpty() }
    val actionsReady = !hasDraft || previewAccepted != null
    val renderedSnapshot = previewAccepted?.snapshot ?: snapshot
    LaunchedEffect(snapshot.identity, snapshot.revision, snapshot.phase, hasDraft, draftRevision, draftGeneration, unresolved, pending, restoring, previewRetry) {
        candidate = null
        previewFailure = null
        if (!hasDraft || composing.isNotEmpty() || unresolved != null || pending || restoring || snapshot.phase != UiSurfacePhase.Ready) return@LaunchedEffect
        if (draftRevision != snapshot.revision) { previewFailure = context.getString(R.string.ui_edit_revision_changed); return@LaunchedEffect }
        val input = UiPreviewInput(snapshot.identity, snapshot.revision,
            drafts.map { (key, value) -> UiEdit.Set(key.first, key.second, value) }, draftGeneration)
        val baseline = snapshot
        delay(180)
        try {
            val result = checkNotNull(currentPreview) { context.getString(R.string.ui_preview_unavailable) }(input)
            result.requireMatches(input, baseline)
            if (latest.identity == input.identity && latest.revision == input.expected_surface_revision &&
                draftGeneration == input.draft_generation && composing.isEmpty()) candidate = result
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) { if (draftGeneration == input.draft_generation) previewFailure = error.message ?: context.getString(R.string.ui_preview_failed) }
    }
    LaunchedEffect(snapshot.identity, restoreAttempt) {
        if (restoring) {
            restoreFailed = false
            try {
                currentRestore(snapshot.identity)?.let { unresolved = it }
                restoring = false
                failure = null
            }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { restoreFailed = true; failure = error.message ?: context.getString(R.string.ui_restore_failed) }
        }
    }
    val enabled = !restoring && !pending && unresolved == null && snapshot.phase == UiSurfacePhase.Ready
    val deliver: (UiSubmission) -> Unit = { input ->
        if (!pending && input.interaction.identity == latest.identity) {
            pending = true
            failure = null
            // Retain before transport, including while the coroutine is cancelled.
            unresolved = input
            scope.launch {
                try {
                    val result = currentSubmit(input)
                    if (input.interaction.identity == latest.identity) when (result) {
                        is UiSubmissionOutcome.Committed -> {
                            if (result.receipt.snapshot.identity == latest.identity) {
                                drafts.clear()
                                draftRevision = null
                                unresolved = null
                                currentAccepted(result.receipt)
                            } else failure = context.getString(R.string.ui_receipt_owner_mismatch)
                        }
                        is UiSubmissionOutcome.Rejected -> { unresolved = null; failure = result.message }
                        is UiSubmissionOutcome.Unknown -> failure = result.message
                    }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    failure = error.message ?: context.getString(R.string.ui_submission_unconfirmed)
                } finally { pending = false }
            }
        }
    }
    val dispatch: (UiActionRef) -> Unit = { action ->
        if (enabled && !actionsReady) previewFailure = context.getString(R.string.ui_preview_wait)
        else if (enabled && composing.isNotEmpty()) failure = context.getString(R.string.ui_finish_composition)
        else if (enabled) {
            val valid = drafts.keys.all { (node, binding) -> latest.nodes[node]?.bindings?.containsKey(binding) == true }
            if (!valid) failure = context.getString(R.string.ui_edit_nodes_changed)
            else if (draftRevision != null && draftRevision != latest.revision) {
                failure = context.getString(R.string.ui_edit_review_version)
            }
            else deliver(UiSubmission(UiInteraction(latest.identity, draftRevision ?: latest.revision,
                drafts.map { (key, value) -> UiEdit.Set(key.first, key.second, value) }, action),
                UUID.randomUUID().toString(), System.currentTimeMillis()))
        }
    }
    val render: @Composable (String) -> Unit = { id ->
        GenerativeUiNode(id, renderedSnapshot, enabled,
            value = { node, binding, committed -> drafts[node to binding] ?: committed },
            edit = { node, binding, value ->
                if (enabled && latest.nodes[node]?.bindings?.containsKey(binding) == true) {
                    if (draftRevision == null) draftRevision = latest.revision
                    drafts[node to binding] = value
                    draftGeneration += 1
                    candidate = null
                }
            }, dispatch = dispatch, present = { id -> if (modal == null) modal = id },
            composition = { id, active ->
                if ((composing[id] == true) != active) {
                    if (active) composing[id] = true else composing.remove(id)
                    draftGeneration += 1
                    candidate = null
                }
            }, actionsEnabled = actionsReady && composing.isEmpty())
    }
    Column(Modifier.semantics { this[UiSurfaceIdentitySemantics] = snapshot.identity },
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        renderedSnapshot.root_instance_id?.let { render(it) }
            ?: snapshot.summary.takeIf { it.isNotEmpty() }?.let { Text(it) }
        if (pending) LinearProgressIndicator(Modifier.fillMaxWidth())
        failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (restoreFailed) TextButton(onClick = { restoreAttempt += 1 }) {
            Text(context.getString(R.string.ui_restore_retry))
        }
        previewFailure?.let { message ->
            Text(message, color = MaterialTheme.colorScheme.error)
            TextButton(enabled = enabled && composing.isEmpty(), onClick = {
                if (editing.revalidate(latest, !enabled || composing.isNotEmpty())) {
                    candidate = null
                    previewRetry += 1
                } else {
                    failure = context.getString(R.string.ui_edit_preserved)
                }
            }) { Text(context.getString(R.string.ui_revalidate)) }
        }
        if (!pending && unresolved == null && drafts.isNotEmpty() && (failure != null || draftRevision != snapshot.revision)) {
            TextButton(onClick = { drafts.clear(); draftRevision = null; failure = null }) {
                Text(context.getString(R.string.ui_discard_edits))
            }
        }
        if (snapshot.phase == UiSurfacePhase.Ready || snapshot.phase == UiSurfacePhase.Pending) unresolved?.let { input -> TextButton(enabled = !pending, onClick = { deliver(input) }) { Text(context.getString(R.string.ui_check_submission)) } }
    }
    LaunchedEffect(snapshot.identity, snapshot.phase, renderedSnapshot.nodes.keys) {
        if (snapshot.phase != UiSurfacePhase.Ready || modal !in renderedSnapshot.nodes) modal = null
    }
    modal?.let { id ->
        Dialog(onDismissRequest = { modal = null }) {
            Surface(shape = MaterialTheme.shapes.large) {
                Column(Modifier.heightIn(max = maximumDialogHeight).padding(20.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    TextButton(onClick = { modal = null }) { Text(context.getString(R.string.common_close)) }
                    renderedSnapshot.nodes[id]?.slots?.get("content").orEmpty().forEach { render(it) }
                }
            }
        }
    }
}
