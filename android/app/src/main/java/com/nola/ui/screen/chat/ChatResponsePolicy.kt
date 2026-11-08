// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Final assistant response presentation policy.

package com.nola.ui.screen.chat

internal fun finalAssistantResponseText(
    response: String,
    hasToolEvents: Boolean,
    emptyResponseText: String
): String {
    val trimmed = response.trim()
    if (trimmed.isNotEmpty()) return trimmed
    return if (hasToolEvents) emptyResponseText else ""
}
