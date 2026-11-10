package com.nola.ui.screen.chat

import com.nola.data.local.*
import com.nola.domain.projection.ConversationProjectionV1 as Projection
import org.junit.Assert.*
import org.junit.Test

class CanonicalMessagePartPresentationTest {
    private fun reasoning(id: String) = Projection.Part.Reasoning(id, 1, Projection.ActivityState.Completed, "same actual summary")
    private fun surface() = Projection.Part.UiSurface("ui", 1, UiSurfaceRevisionRef(UiSurfaceIdentity(
        UiSurfaceOwner("host", "conversation", "participant", "context", "run", "assistant", "ui"), "card", 2), 3), "Card")

    @Test fun actualJournalOrderSurvivesSurfaceAndEmptyTextBoundaries() {
        val tool = Projection.Part.Tool("tool", 1, "call", "render_ui", com.nola.domain.projection.ToolExecutionSource.Model,
            Projection.ActivityState.Completed, null, kotlinx.serialization.json.JsonNull, null, emptyList())
        val parts = listOf(reasoning("before"), tool, surface(), reasoning("after"),
            Projection.Part.Text("answer", 1, "result", true), Projection.Part.Text("placeholder", 1, "", false))
        val blocks = canonicalMessageBlocks(parts)
        assertEquals(listOf("before", "ui", "after", "answer", "placeholder"), blocks.map { it.key })
        assertEquals(listOf("before", "tool"), (blocks[0] as CanonicalMessageBlock.Activity).parts.map { it.partId })
        assertSame(parts[3], (blocks[2] as CanonicalMessageBlock.Activity).parts.single())
        assertTrue(blocks[4] is CanonicalMessageBlock.Part)
    }

    @Test fun equalSummariesKeepDistinctStableStepsWithoutCrossingText() {
        val parts = listOf(reasoning("one"), reasoning("two"), Projection.Part.Text("empty", 1, "", false), reasoning("three"))
        val blocks = canonicalMessageBlocks(parts)
        assertEquals(listOf("one", "empty", "three"), blocks.map { it.key })
        assertEquals(listOf("one", "two"), (blocks[0] as CanonicalMessageBlock.Activity).parts.map { it.partId })
    }

    @Test fun surfaceRequiresExactConversationAndMessageAndKeepsGenerationAndVersion() {
        val part = surface()
        val actual = checkNotNull(uiReferencePresentation(part, "conversation", "assistant"))
        assertEquals(part.surface, actual.reference)
        assertFalse(actual.historical)
        assertThrows(IllegalArgumentException::class.java) { uiReferencePresentation(part, "parent", "assistant") }
        assertThrows(IllegalArgumentException::class.java) { uiReferencePresentation(part, "conversation", "different-assistant") }
    }

    @Test fun messageOwnedChoicesSurviveSegmentPrependAndManualCollapseWinsConflictingMembers() {
        val original = reasoning("original")
        val inserted = reasoning("inserted")
        val choices = CanonicalActivityDisclosureChoices().details(listOf(original), original.partId, true)
        assertTrue(original.partId in choices.detailsPartIds)
        assertEquals(true, choices.expandedFor(listOf(inserted, original)))
        assertFalse(inserted.partId in choices.detailsPartIds)
        val conflicting = choices.expanded(listOf(inserted), false)
        assertEquals(false, conflicting.expandedFor(listOf(original, inserted)))
        assertEquals(true, conflicting.expanded(listOf(inserted, original), true).expandedFor(listOf(original, inserted)))
    }

    @Test fun unapprovedTextIsNotRevealedAtItsOriginalPositionOrInCopy() {
        val hidden = Projection.Part.Text("held", 1, "private withheld text", true, com.nola.domain.projection.TextReviewState.Pending)
        assertEquals("", hidden.disclosedMarkdown)
        assertEquals("", executionAnswer(listOf(hidden), "stale withheld text"))
        assertEquals("held", canonicalMessageBlocks(listOf(hidden)).single().key)
    }
}
