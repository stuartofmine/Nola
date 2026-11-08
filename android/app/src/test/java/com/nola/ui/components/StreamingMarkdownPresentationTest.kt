package com.nola.ui.components

import com.nola.agent.trace.streamingMarkdownProjection
import com.nola.agent.trace.StreamingMarkdownProjectionCache
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamingMarkdownPresentationTest {
    @Test(timeout = 1_000)
    fun partialTableRowsAndBlockMarkersRemainReadable() {
        val table = parseMarkdownBlocks("| A | B |\n|---|---|\n| first cell").single() as MarkdownBlock.Table
        assertEquals(listOf("A", "B"), table.header)
        assertEquals(listOf(listOf("first cell")), table.rows)
        assertEquals(MarkdownBlock.Paragraph(listOf(MarkdownInline.Text("#"))), parseMarkdownBlocks("#").single())
    }

    @Test
    fun appendOnlyLongStreamReprojectsOnlyTheMutableSuffix() {
        val cache = StreamingMarkdownProjectionCache()
        val initial = buildString {
            repeat(2_000) { append("A stable paragraph number $it.\n\n") }
        }
        cache.project(initial, complete = false)

        val appended = initial + "next token"
        val incremental = cache.project(appended, complete = false)
        val cold = streamingMarkdownProjection(appended, complete = false)

        assertEquals(cold.visibleText, incremental.visibleText)
        assertEquals(cold.committedBlocks.map { it.id }, incremental.committedBlocks.map { it.id })
        assertTrue(cache.lastParsedCharacterCount < 128)
    }

    @Test
    fun correctionInsideCommittedPrefixInvalidatesIncrementalProjection() {
        val cache = StreamingMarkdownProjectionCache()
        val initial = "First stable paragraph.\n\nSecond stable paragraph.\n\nMutable"
        cache.project(initial, complete = false)

        val corrected = initial.replace("First", "Corrected first")
        val incremental = cache.project(corrected, complete = false)
        val cold = streamingMarkdownProjection(corrected, complete = false)

        assertEquals(cold, incremental)
        assertEquals(corrected.length, cache.lastParsedCharacterCount)
    }

    @Test
    fun incompleteInlineMarkdownIsRenderedImmediatelyWithAStableTail() {
        assertEquals("**Partial**", stabilizeStreamingMarkdownTail("**Partial"))
    }

    @Test
    fun canonicalCoreTextIsNotReplayedThroughASecondClientQueue() {
        val source = "## Result\n\nA stable sentence.\n\n```swift\nlet value = 1"
        val projection = streamingMarkdownProjection(source, complete = false)
        assertEquals(
            source,
            projection.committedBlocks.joinToString("") { it.markdown } +
                projection.mutableTail?.markdown.orEmpty(),
        )
    }

    @Test
    fun openFencedCodeStreamsBeforeFenceCloses() {
        val openFence = "```swift\nlet value = 1"
        assertEquals(openFence, stabilizeStreamingMarkdownTail(openFence))
    }

    @Test
    fun paragraphSoftBreaksRemainVisibleAfterMarkdownParsing() {
        val blocks = parseMarkdownBlocks("FIRST\nSECOND\nTHIRD")
        val paragraph = blocks.single() as MarkdownBlock.Paragraph

        assertEquals(
            listOf(
                MarkdownInline.Text("FIRST"),
                MarkdownInline.SoftBreak,
                MarkdownInline.Text("SECOND"),
                MarkdownInline.SoftBreak,
                MarkdownInline.Text("THIRD"),
            ),
            paragraph.inlines,
        )
    }

    @Test
    fun continuousParagraphDoesNotBecomeSeparateSentenceViews() {
        val source = "这是一段需要保持舒缓节奏的正文，即使中间出现逗号也不应立刻切开。下一句继续完整呈现。"
        val streaming = streamingMarkdownProjection(source, complete = false)
        val completed = streamingMarkdownProjection(source, complete = true)
        val completedText = completed.committedBlocks.joinToString("") { it.markdown }

        assertEquals(source, completedText)
        assertEquals(
            streaming.committedBlocks.map { it.id },
            completed.committedBlocks.take(streaming.committedBlocks.size).map { it.id },
        )
        assertEquals(0, streaming.committedBlocks.size)
        assertEquals(1, completed.committedBlocks.size)
        assertEquals(streaming.mutableTail?.id, completed.committedBlocks.single().id)
    }

    @Test
    fun listsAndTableRowsKeepOneOwnerAcrossEveryChunkAndCompletion() {
        val cache = StreamingMarkdownProjectionCache()
        val source = "1. First item\n2. Second item\n\n| A | B |\n| --- | --- |\n| one | two |"
        for (end in 1..source.length) {
            val live = cache.project(source.take(end), complete = false)
            assertEquals(source.take(end), live.visibleText)
            assertEquals(streamingMarkdownProjection(source.take(end), false), live)
        }
        val live = cache.project(source, false)
        val final = cache.project(source, true)
        assertEquals(1, live.committedBlocks.size)
        assertEquals(2, final.committedBlocks.size)
        assertEquals(live.mutableTail?.id, final.committedBlocks.last().id)
        assertEquals(source, final.visibleText)
        assertEquals(source + " another cell", cache.project(source + " another cell", false).visibleText)
    }

    @Test
    fun mixedOrShorterFencesCannotCommitTheMiddleOfACodeBlock() {
        val source = "````markdown\n```kotlin\nval x = 1\n```\n\n~~~\n\n````\n\nAfter"
        val projection = streamingMarkdownProjection(source, false)
        assertEquals(1, projection.committedBlocks.size)
        assertTrue(projection.committedBlocks.single().markdown.endsWith("````\n\n"))
        assertEquals("After", projection.mutableTail?.markdown)
    }

    @Test
    fun singleNewlineNeverDetachesAListOrTableHeader() {
        for (source in listOf("1. First\n", "| A | B |\n", "Paragraph.\n", "- item\n")) {
            val projection = streamingMarkdownProjection(source, false)
            assertTrue(projection.committedBlocks.isEmpty())
            assertEquals(source, projection.mutableTail?.markdown)
        }
    }
}
