package com.nola.ui.generative

import com.nola.data.local.*
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Test
import kotlin.test.*

class UiDraftStateTest {
    private val identity = UiSurfaceIdentity(UiSurfaceOwner("host", "conversation", "agent", "context", "run", "message", "part"), "form", 1)
    private fun snapshot(bindings: Map<String, String> = mapOf("value" to "/name")) = UiSnapshot(
        identity, 2, "catalog", "digest", UiSurfacePhase.Ready, "field",
        mapOf("field" to UiNodeSnapshot("field", "field", "TextField", emptyMap(), emptyMap(), bindings, emptyMap(), emptyList())), "form")
    private fun draft() = UiDraftState().apply {
        revision.value = 1
        drafts["field" to "value"] = JsonPrimitive("保留输入")
    }

    @Test fun explicitConfirmationRebasesWithoutDiscardingInput() {
        val state = draft()
        val generation = state.generation.longValue
        assertTrue(state.revalidate(snapshot(), false))
        assertEquals(2L, state.revision.value)
        assertEquals(JsonPrimitive("保留输入"), state.drafts["field" to "value"])
        assertTrue(state.generation.longValue > generation)
    }

    @Test fun removedFieldsKeepDraftAndOriginalRevision() {
        val state = draft()
        assertFalse(state.revalidate(snapshot(emptyMap()), false))
        assertEquals(1L, state.revision.value)
        assertEquals(1, state.drafts.size)
    }

    @Test fun compositionPendingUnknownAndClosedCannotRebase() {
        val state = draft()
        assertFalse(state.revalidate(snapshot(), true))
        state.unresolved.value = UiSubmission(UiInteraction(identity, 1, emptyList(), null), "request", 1)
        assertFalse(state.revalidate(snapshot(), false))
        state.unresolved.value = null
        assertFalse(state.revalidate(snapshot().copy(phase = UiSurfacePhase.ReadOnly), false))
        assertEquals(1L, state.revision.value)
    }

    @Test fun coldStartRetainsValuesAndOriginalRequestAfterRejection() {
        val state = UiDraftState()
        val submission = UiSubmission(UiInteraction(identity, 1,
            listOf(UiEdit.Set("field", "value", JsonPrimitive("保留填写值 👩🏽‍💻"))), null), "original-request", 1)
        state.restore(submission, identity)
        assertEquals(submission, state.unresolved.value)
        state.unresolved.value = null
        assertEquals(1L, state.revision.value)
        assertEquals(JsonPrimitive("保留填写值 👩🏽‍💻"), state.drafts["field" to "value"])
    }

    @Test fun foreignRestoreCannotOverwriteExistingDraft() {
        val state = draft()
        val submission = UiSubmission(UiInteraction(identity.copy(surface_id = "foreign"), 2, emptyList(), null), "request", 1)
        assertFailsWith<IllegalArgumentException> { state.restore(submission, identity) }
        assertEquals(JsonPrimitive("保留输入"), state.drafts["field" to "value"])
        assertNull(state.unresolved.value)
    }
}
