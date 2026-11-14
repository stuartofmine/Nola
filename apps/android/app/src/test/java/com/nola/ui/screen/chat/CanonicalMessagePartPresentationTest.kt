package com.nola.ui.screen.chat

import com.nola.data.local.*
import com.nola.domain.projection.ConversationProjectionV1 as Projection
import org.junit.Assert.*
import org.junit.Test

class CanonicalMessagePartPresentationTest {
    @org.junit.Test fun liveProcessDoesNotBecomeHistoryBetweenSteps() {
        org.junit.Assert.assertEquals("Processing", executionProcessHeading(true, null, "Processing", "History"))
        org.junit.Assert.assertEquals("Reading", executionProcessHeading(true, "Reading", "Processing", "History"))
        org.junit.Assert.assertEquals("History", executionProcessHeading(false, "Stale", "Processing", "History"))
    }

    @Test fun sourceAliasesCollapseButDistinctUrisRemainInBibliography() {
        val source = Projection.Part.Source("source", 1, "s", "Title", "https://example.com", null, null)
        val alias = source.copy(partId = "alias")
        val other = source.copy(partId = "other", sourceId = "other", uri = "https://example.com/other")
        assertEquals(listOf("source", "other"), canonicalBibliography(listOf(source, alias, other)).map { it.first.partId })
    }

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

    @Test fun recoveredFailureStaysInHistoryAndDoesNotBecomeCurrentActivity() {
        fun tool(id: String, state: Projection.ActivityState) = Projection.Part.Tool(id, 1, id, "bash",
            com.nola.domain.projection.ToolExecutionSource.Model, state, null,
            kotlinx.serialization.json.JsonNull, null, emptyList(), title = id)
        val failed = tool("failed", Projection.ActivityState.Failed)
        val retry = tool("retry", Projection.ActivityState.Running)
        val parts = listOf(failed, retry)
        assertSame(retry, currentExecutionActivity(parts, true))
        assertNull(CanonicalActivityDisclosureChoices().expandedFor(parts))
        val settled = listOf(failed, tool("retry", Projection.ActivityState.Completed))
        assertNull(currentExecutionActivity(settled, true))
        assertNull(currentExecutionActivity(parts, false))
        assertEquals(listOf("failed", "retry"), (canonicalMessageBlocks(settled).single() as CanonicalMessageBlock.Activity).parts.map { it.partId })
        val parallel = parts + tool("other-completed", Projection.ActivityState.Completed)
        assertSame(retry, currentExecutionActivity(parallel, true))
    }

    @Test fun unapprovedTextIsNotRevealedAtItsOriginalPositionOrInCopy() {
        val hidden = Projection.Part.Text("held", 1, "private withheld text", true, com.nola.domain.projection.TextReviewState.Pending)
        assertEquals("", hidden.disclosedMarkdown)
        assertEquals("", executionAnswer(listOf(hidden), "stale withheld text"))
        assertEquals("held", canonicalMessageBlocks(listOf(hidden)).single().key)
    }
}
