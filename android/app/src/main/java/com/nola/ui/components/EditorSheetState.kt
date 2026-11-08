package com.nola.ui.components

import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.SheetState
import androidx.compose.material3.SheetValue
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState

internal enum class EditorDismissDecision { Allow, Busy, ConfirmDiscard }

internal fun editorDismissDecision(saving: Boolean, dirty: Boolean): EditorDismissDecision = when {
    saving -> EditorDismissDecision.Busy
    dirty -> EditorDismissDecision.ConfirmDiscard
    else -> EditorDismissDecision.Allow
}

/** A swipe must not hide an editor before the user can decide to keep editing. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun rememberGuardedEditorSheetState(
    saving: Boolean,
    dirty: Boolean,
    onDiscardRequested: () -> Unit,
): SheetState {
    val currentSaving by rememberUpdatedState(saving)
    val currentDirty by rememberUpdatedState(dirty)
    val requestDiscard by rememberUpdatedState(onDiscardRequested)
    return rememberModalBottomSheetState(
        skipPartiallyExpanded = true,
        confirmValueChange = { target ->
            if (target != SheetValue.Hidden) true
            else when (editorDismissDecision(currentSaving, currentDirty)) {
                EditorDismissDecision.Allow -> true
                EditorDismissDecision.Busy -> false
                EditorDismissDecision.ConfirmDiscard -> { requestDiscard(); false }
            }
        },
    )
}
