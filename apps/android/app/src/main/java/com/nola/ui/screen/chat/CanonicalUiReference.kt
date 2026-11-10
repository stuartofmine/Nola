package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import com.nola.R
import com.nola.data.local.*
import com.nola.ui.generative.*

/** All actions and reads retain the exact Core owner/reference, including child contexts. */
@Composable
internal fun CanonicalUiReference(
    part: UiReferencePresentation,
    snapshots: List<UiSnapshot>,
    readHistory: suspend (UiSurfaceRevisionRef) -> UiSnapshot?,
    submit: suspend (UiSubmission) -> UiSubmissionOutcome,
    accepted: (UiInteractionReceipt) -> Unit,
    restore: suspend (UiSurfaceIdentity) -> UiSubmission?,
    preview: (suspend (UiPreviewInput) -> UiPreview)?,
) {
    GenerativeUiReference(part.reference, part.summary,
        if (part.historical) null else snapshots.singleOrNull { it.identity == part.reference.identity },
        readHistory, submit, accepted, restore, !part.historical, preview)
}

/** A reader without an owning surface port cannot invent a live surface or action. */
@Composable
internal fun CanonicalUiReferenceUnavailable(part: UiReferencePresentation) {
    Column {
        if (part.summary.isNotBlank()) Text(part.summary)
        Text(stringResource(R.string.ui_version_readonly, part.reference.revision.toString()))
        Text(stringResource(R.string.ui_version_unavailable))
    }
}
