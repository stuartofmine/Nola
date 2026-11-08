// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class TaggedThinkingStreamParserTest {
    @Test
    fun splitTagsDoNotLeakReasoningIntoAnswer() {
        val parser = TaggedThinkingStreamParser()
        val deltas = listOf("<thi", "nk>分析", "过程</thi", "nk>最终", "答案")
            .map(parser::accept)

        assertEquals("分析过程", deltas.joinToString("") { it.reasoning })
        assertEquals("最终答案", deltas.joinToString("") { it.answer })
    }

    @Test
    fun plainMarkdownStreamsDirectlyAsAnswer() {
        val parser = TaggedThinkingStreamParser()
        val deltas = listOf("## 标题\n", "正文").map(parser::accept)

        assertEquals("", deltas.joinToString("") { it.reasoning })
        assertEquals("## 标题\n正文", deltas.joinToString("") { it.answer })
    }

    @Test
    fun closingTagAndAnswerInOneChunkAreSeparatedOnce() {
        val parser = TaggedThinkingStreamParser()
        val first = parser.accept("<think>reasoning</think>answer")
        val second = parser.accept(" continued")

        assertEquals("reasoning", first.reasoning)
        assertEquals("answer continued", first.answer + second.answer)
    }
}
