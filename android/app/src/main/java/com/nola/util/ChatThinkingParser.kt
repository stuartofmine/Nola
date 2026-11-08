package com.nola.util

import javax.inject.Inject
import javax.inject.Singleton

data class ParsedResponse(
    val thinking: String?,
    val content: String,
    val hasThinking: Boolean
)

@Singleton
class ChatThinkingParser @Inject constructor() {

    fun parse(text: String, isComplete: Boolean): ParsedResponse {
        val thinkStart = text.indexOf("<think>")
        val thinkEnd = text.indexOf("</think>")

        return when {
            thinkStart < 0 && thinkEnd < 0 -> {
                // No thinking block
                ParsedResponse(thinking = null, content = text, hasThinking = false)
            }
            thinkEnd >= 0 -> {
                // Thinking block is complete
                val thinking = if (thinkStart >= 0) {
                    text.substring(thinkStart + 7, thinkEnd).trim()
                } else {
                    text.substring(0, thinkEnd).trim()
                }
                val content = text.substring(thinkEnd + 8).trimStart()
                ParsedResponse(thinking = thinking, content = content, hasThinking = true)
            }
            thinkStart >= 0 -> {
                // Still inside thinking block
                val thinking = text.substring(thinkStart + 7).trim()
                ParsedResponse(
                    thinking = thinking,
                    content = "",
                    hasThinking = true
                )
            }
            else -> {
                ParsedResponse(thinking = null, content = text, hasThinking = false)
            }
        }
    }
}
