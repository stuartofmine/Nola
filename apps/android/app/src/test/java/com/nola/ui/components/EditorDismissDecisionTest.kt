package com.nola.ui.components

import org.junit.Assert.assertEquals
import org.junit.Test

class EditorDismissDecisionTest {
    @Test fun aSaveInFlightCannotBeDismissedEvenWhenTheDraftIsClean() {
        assertEquals(EditorDismissDecision.Busy, editorDismissDecision(saving = true, dirty = false))
        assertEquals(EditorDismissDecision.Busy, editorDismissDecision(saving = true, dirty = true))
    }

    @Test fun unsavedChangesRequireConfirmationRatherThanHidingTheSheet() {
        assertEquals(EditorDismissDecision.ConfirmDiscard, editorDismissDecision(saving = false, dirty = true))
    }

    @Test fun cleanIdleEditorCanBeDismissed() {
        assertEquals(EditorDismissDecision.Allow, editorDismissDecision(saving = false, dirty = false))
    }
}
