// SPDX-License-Identifier: Apache-2.0
package com.nola.agent.trace

data class StreamingMarkdownBlock(
    val id: String,
    val markdown: String,
    val committed: Boolean,
)

data class StreamingMarkdownProjection(
    val committedBlocks: List<StreamingMarkdownBlock> = emptyList(),
    val mutableTail: StreamingMarkdownBlock? = null,
) {
    val visibleText: String
        get() = buildString {
            committedBlocks.forEach { append(it.markdown) }
            mutableTail?.let { append(it.markdown) }
        }
}

/** Stable Markdown paragraphs plus one live tail, matching the Apple clients.
 * Completion promotes that tail in place; it never replays or re-splits prose.
 */
class StreamingMarkdownProjectionCache {
    private var previous = StreamingMarkdownProjection()
    private var previousSource = ""
    private var previousCommittedLength = 0

    internal var lastParsedCharacterCount: Int = 0
        private set

    fun project(markdown: String, complete: Boolean): StreamingMarkdownProjection {
        val canReuse = markdown.length >= previousCommittedLength && markdown.regionMatches(
            thisOffset = 0, other = previousSource, otherOffset = 0, length = previousCommittedLength,
        )
        val prefix = if (canReuse) previous.committedBlocks else emptyList()
        val offset = if (canReuse) previousCommittedLength else 0
        lastParsedCharacterCount = markdown.length - offset
        // Retain the open-tail partition even after completion. If an accepted
        // correction or continuation arrives, its paragraph keeps the same owner.
        val live = partitionMarkdown(markdown.substring(offset), offset, prefix)
        previous = live
        previousSource = markdown
        previousCommittedLength = markdown.length - live.mutableTail?.markdown.orEmpty().length
        return if (complete) live.settle() else live
    }
}

fun streamingMarkdownProjection(markdown: String, complete: Boolean): StreamingMarkdownProjection {
    val live = partitionMarkdown(markdown, 0, emptyList())
    return if (complete) live.settle() else live
}

private fun StreamingMarkdownProjection.settle(): StreamingMarkdownProjection =
    StreamingMarkdownProjection(committedBlocks = committedBlocks + listOfNotNull(mutableTail?.copy(committed = true)))

private fun partitionMarkdown(
    source: String,
    sourceOffset: Int,
    committedPrefix: List<StreamingMarkdownBlock>,
): StreamingMarkdownProjection {
    val blocks = committedPrefix.toMutableList()
    var start = 0
    var index = 0
    var fenceCharacter: Char? = null
    var fenceLength = 0
    while (index < source.length) {
        val newline = source.indexOf('\n', index)
        val lineEnd = if (newline < 0) source.length else newline + 1
        val trimmed = source.substring(index, lineEnd).trim()
        val marker = trimmed.firstOrNull()?.takeIf { it == '`' || it == '~' }
        val length = marker?.let { character -> trimmed.takeWhile { it == character }.length } ?: 0
        if (length >= 3) {
            if (fenceCharacter == null) {
                fenceCharacter = marker
                fenceLength = length
            } else if (marker == fenceCharacter && length >= fenceLength && trimmed.drop(length).isBlank()) {
                fenceCharacter = null
                fenceLength = 0
            }
        }
        // A trailing single newline is not a blank line. Tables, lists and
        // fenced examples must keep their Markdown context across deltas.
        if (fenceCharacter == null && trimmed.isEmpty() && newline >= 0 && source.substring(start, lineEnd).isNotBlank()) {
            blocks += StreamingMarkdownBlock("block-${sourceOffset + start}", source.substring(start, lineEnd), committed = true)
            start = lineEnd
        }
        index = lineEnd
    }
    return StreamingMarkdownProjection(
        committedBlocks = blocks,
        mutableTail = source.substring(start).takeIf { it.isNotEmpty() }?.let {
            StreamingMarkdownBlock("block-${sourceOffset + start}", it, committed = false)
        },
    )
}
