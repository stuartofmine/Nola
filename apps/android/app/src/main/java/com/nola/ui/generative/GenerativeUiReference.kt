package com.nola.ui.generative

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import com.nola.R
import com.nola.data.local.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** A message keeps an exact version anchor while the active card follows Core. */
@Composable
fun GenerativeUiReference(
    reference: UiSurfaceRevisionRef,
    summary: String,
    current: UiSnapshot?,
    readHistory: suspend (UiSurfaceRevisionRef) -> UiSnapshot?,
    submit: suspend (UiSubmission) -> UiSubmissionOutcome,
    accepted: (UiInteractionReceipt) -> Unit,
    restore: suspend (UiSurfaceIdentity) -> UiSubmission? = { null },
    recoverable: Boolean = true,
    preview: (suspend (UiPreviewInput) -> UiPreview)? = null,
) {
    val context = LocalContext.current
    val activeCurrent = current?.phase == UiSurfacePhase.Ready || current?.phase == UiSurfacePhase.Pending
    var unresolved by remember(reference.identity) { mutableStateOf<UiSubmission?>(null) }
    var recovering by remember(reference.identity) { mutableStateOf(false) }
    var recoveryFailure by remember(reference.identity) { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(reference.identity, activeCurrent, recoverable) {
        if (!activeCurrent && recoverable) {
            try { unresolved = restore(reference.identity) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { recoveryFailure = error.message }
        }
    }
    var expanded by remember(reference.identity) { mutableStateOf(false) }
    var history by remember(reference) { mutableStateOf<UiSnapshot?>(null) }
    var failure by remember(reference) { mutableStateOf<String?>(null) }
    var loading by remember(reference) { mutableStateOf(false) }
    LaunchedEffect(reference, expanded) {
        if (!expanded) return@LaunchedEffect
        loading = true; history = null; failure = null
        try {
            val value = readHistory(reference)
            require(value == null || value.identity == reference.identity && value.revision == reference.revision &&
                (value.phase == UiSurfacePhase.ReadOnly || value.phase == UiSurfacePhase.Deleted) &&
                value.nodes.values.all { it.actions.isEmpty() && it.bindings.isEmpty() })
            history = value
            if (value == null) failure = context.getString(R.string.ui_version_unavailable)
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) { failure = error.message ?: context.getString(R.string.ui_version_read_failed) }
        finally { loading = false }
    }
    Column {
        if (current != null) GenerativeUiSurface(current, submit, accepted, restore, preview)
        else if (summary.isNotBlank()) Text(summary)
        if (!activeCurrent) unresolved?.let { input ->
            TextButton(enabled = !recovering, onClick = {
                recovering = true
                scope.launch {
                    try {
                        when (val result = submit(input)) {
                            is UiSubmissionOutcome.Committed -> {
                                if (result.receipt.snapshot.identity == input.interaction.identity) {
                                    unresolved = null; accepted(result.receipt)
                                } else recoveryFailure = context.getString(R.string.ui_receipt_owner_mismatch)
                            }
                            is UiSubmissionOutcome.Rejected -> { unresolved = null; recoveryFailure = result.message }
                            is UiSubmissionOutcome.Unknown -> recoveryFailure = result.message
                        }
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (error: Exception) { recoveryFailure = error.message }
                    finally { recovering = false }
                }
            }) { Text(context.getString(R.string.ui_submission_check)) }
        }
        recoveryFailure?.let { Text(it) }
        if (current == null) TextButton(onClick = { expanded = !expanded }) { Text(context.getString(R.string.ui_version_view)) }
        if (current == null && expanded) {
            Text(context.getString(R.string.ui_version_readonly))
            if (loading) Text(context.getString(R.string.ui_version_loading))
            failure?.let { Text(it) }
            history?.let { GenerativeUiSurface(it, { UiSubmissionOutcome.Rejected(context.getString(R.string.ui_version_not_editable)) }, {}) }
        }
    }
}
