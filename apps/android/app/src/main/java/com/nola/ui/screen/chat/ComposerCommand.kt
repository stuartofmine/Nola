package com.nola.ui.screen.chat

import com.nola.R

// Explicit local UI intents. Unknown commands never enter the conversation queue.
internal enum class ComposerCommand(val label: Int, val acceptsArgument: Boolean = false) {
    model(R.string.tab_models), experts(R.string.chat_experts), resources(R.string.inspector_resources),
    permissions(R.string.chat_permission_mode), plan(R.string.workspace_plans),
    search(R.string.common_search, true), stop(R.string.chat_stop), help(R.string.composer_commands);

    companion object {
        fun query(text: String): String? = text.takeIf { it.startsWith('/') && it.drop(1).none { char -> char.isWhitespace() || char == '/' } }?.drop(1)?.lowercase()
        fun parse(text: String): Pair<ComposerCommand?, String>? {
            if (!text.startsWith('/')) return null
            val parts = text.drop(1).trim().split(Regex("\\s+"))
            val command = entries.find { it.name == parts.first().lowercase() }
            val argument = parts.drop(1).joinToString(" ")
            return Pair(command?.takeIf { it.acceptsArgument || argument.isEmpty() }, argument)
        }
    }
}
