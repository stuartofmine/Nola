// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

internal data class TaggedThinkingDelta(
    val reasoning: String = "",
    val answer: String = "",
)

/** Incrementally separates optional `<think>...</think>` output from the answer. */
internal class TaggedThinkingStreamParser {
    private enum class Phase { Detecting, Reasoning, Answer }

    private var phase = Phase.Detecting
    private var pending = ""

    fun reset() {
        phase = Phase.Detecting
        pending = ""
    }

    fun accept(chunk: String): TaggedThinkingDelta {
        if (chunk.isEmpty()) return TaggedThinkingDelta()
        pending += chunk
        val reasoning = StringBuilder()
        val answer = StringBuilder()

        while (pending.isNotEmpty()) {
            when (phase) {
                Phase.Detecting -> when {
                    pending.startsWith(OPEN_TAG) -> {
                        pending = pending.removePrefix(OPEN_TAG)
                        phase = Phase.Reasoning
                    }
                    OPEN_TAG.startsWith(pending) -> return TaggedThinkingDelta(
                        reasoning = reasoning.toString(),
                        answer = answer.toString(),
                    )
                    else -> {
                        phase = Phase.Answer
                    }
                }
                Phase.Reasoning -> {
                    val closing = pending.indexOf(CLOSE_TAG)
                    if (closing >= 0) {
                        reasoning.append(pending.take(closing))
                        pending = pending.drop(closing + CLOSE_TAG.length)
                        phase = Phase.Answer
                    } else {
                        val retained = longestSuffixMatchingPrefix(pending, CLOSE_TAG)
                        reasoning.append(pending.dropLast(retained))
                        pending = pending.takeLast(retained)
                        return TaggedThinkingDelta(reasoning.toString(), answer.toString())
                    }
                }
                Phase.Answer -> {
                    answer.append(pending)
                    pending = ""
                }
            }
        }
        return TaggedThinkingDelta(reasoning.toString(), answer.toString())
    }

    private fun longestSuffixMatchingPrefix(value: String, marker: String): Int =
        (minOf(value.length, marker.length - 1) downTo 1)
            .firstOrNull { length -> value.endsWith(marker.take(length)) }
            ?: 0

    private companion object {
        const val OPEN_TAG = "<think>"
        const val CLOSE_TAG = "</think>"
    }
}
