// SPDX-License-Identifier: Apache-2.0

package com.nola.core

import android.content.Context

internal object AndroidSystemPrompt {
    private const val STYLE_ASSET = "nola_response_styles.txt"
    private const val CUSTOM_PROMPT_LIMIT = 6_000

    fun build(
        context: Context,
        responseStyle: String,
        customSystemPrompt: String = "",
        personalContext: String = "",
        builtContext: String = "",
    ): String {
        val stylePrompt = styleSection(loadAsset(context, STYLE_ASSET), responseStyle)
        val fragments = agentFragments(stylePrompt, customSystemPrompt).toMutableList()
        personalContext.trim().takeIf(String::isNotBlank)?.let { memory ->
            fragments += AgentPromptFragment(
                id = "android:personal-context",
                layer = AgentPromptLayer.ConversationContext,
                trust = AgentPromptTrust.Reference,
                cacheable = false,
                content = memory,
            )
        }
        builtContext.trim().takeIf(String::isNotBlank)?.let { contextValue ->
            fragments += AgentPromptFragment(
                id = "android:built-context",
                layer = AgentPromptLayer.ConversationContext,
                trust = AgentPromptTrust.Reference,
                cacheable = false,
                content = contextValue,
            )
        }
        return CoreAgentPromptComposer.compose(fragments).rendered
    }

    fun agentFragments(
        context: Context,
        responseStyle: String,
        customSystemPrompt: String = "",
    ): List<AgentPromptFragment> = agentFragments(
        stylePrompt = styleSection(loadAsset(context, STYLE_ASSET), responseStyle),
        customSystemPrompt = customSystemPrompt,
    )

    internal fun agentFragments(
        stylePrompt: String,
        customSystemPrompt: String = "",
    ): List<AgentPromptFragment> = buildList {
        add(
            AgentPromptFragment(
                id = "android:host-identity",
                layer = AgentPromptLayer.Identity,
                trust = AgentPromptTrust.HostFact,
                cacheable = true,
                content = "The active user-facing surface is Nola for Android.",
            ),
        )
        stylePrompt.trim().takeIf(String::isNotBlank)
            ?.let { style ->
                add(
                    AgentPromptFragment(
                        id = "android:user-response-style",
                        layer = AgentPromptLayer.ResponseStyle,
                        trust = AgentPromptTrust.UserInstruction,
                        cacheable = false,
                        content = style,
                    ),
                )
            }
        normalizedCustomInstructions(customSystemPrompt)
            .takeIf(String::isNotBlank)
            ?.let { instructions ->
                add(
                    AgentPromptFragment(
                        id = "android:user-custom-instructions",
                        layer = AgentPromptLayer.ExecutionPolicy,
                        trust = AgentPromptTrust.UserInstruction,
                        cacheable = false,
                        content = instructions,
                    ),
                )
            }
    }

    internal fun normalizedCustomInstructions(value: String): String = value.trim().take(CUSTOM_PROMPT_LIMIT)

    /** A typed definition fact, not an extraction from an old system prompt. */
    internal fun customInstructionComponent(value: String): AgentCoreContextComponent? =
        normalizedCustomInstructions(value).takeIf(String::isNotBlank)?.let {
            AgentCoreContextComponent(kind = "prompt_template", id = "android:user-custom-instructions",
                content = kotlinx.serialization.json.JsonPrimitive(it),
                source = AgentCoreContextComponentSource(platform = "android", title = "Custom instructions"))
        }

    internal fun styleSection(document: String, style: String): String {
        val marker = "[${style.trim().ifEmpty { "casual" }}]"
        var capturing = false
        val lines = mutableListOf<String>()
        for (line in document.lineSequence()) {
            val trimmed = line.trim()
            if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
                if (capturing) break
                capturing = trimmed == marker
            } else if (capturing) {
                lines += line
            }
        }
        return lines.joinToString("\n").trim().ifEmpty {
            "Use a natural, clear, and direct tone. Preserve the user's language."
        }
    }

    private fun loadAsset(context: Context, name: String): String =
        context.assets.open(name).bufferedReader(Charsets.UTF_8).use { reader ->
            reader.readText().trim().also { content ->
                require(content.isNotEmpty()) { "Required prompt asset is empty: $name" }
            }
        }
}
