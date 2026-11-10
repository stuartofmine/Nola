package com.nola.ui.screen.models

import com.nola.data.local.CanonicalModelProfile
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.domain.model.ModelParameters
import org.junit.Assert.*
import org.junit.Test

class ModelParameterReloadTest {
    private fun snapshot(revision: Long) = ModelParametersStore.ParameterSnapshot(
        ModelParameters(temperature = 0.25f),
        CanonicalModelProfile("fixture", "Fixture", "api", "fixture", "remote", "{}", "{}", false, true, revision, 1, 1),
    )

    @Test
    fun confirmedReloadRefreshesBaselineAndResetsDraftEvenWhenValuesAreUnchanged() {
        val original = snapshot(1)
        val current = ModelsUiState(parameterSnapshot = original, parameters = original.parameters,
            parameterEditorVersion = 7, parameterError = "conflict", savingParameters = true)
        val refreshed = snapshot(2)
        val accepted = current.acceptParameterReload(original, refreshed)
        assertSame(refreshed, accepted.parameterSnapshot)
        assertEquals(8L, accepted.parameterEditorVersion)
        assertEquals(refreshed.parameters, accepted.parameters)
        assertNull(accepted.parameterError)
        assertFalse(accepted.savingParameters)
        assertEquals("conflict", current.parameterError)
    }

    @Test
    fun staleReadCannotChangeAnotherEditorOrReopenClosedEditor() {
        val original = snapshot(1)
        val current = ModelsUiState(parameterSnapshot = snapshot(1), parameterEditorVersion = 3)
        assertSame(current, current.acceptParameterReload(original, snapshot(2)))
        val closed = ModelsUiState()
        assertSame(closed, closed.acceptParameterReload(original, snapshot(2)))
    }
}
