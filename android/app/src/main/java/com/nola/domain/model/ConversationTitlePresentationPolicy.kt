package com.nola.domain.model

object ConversationTitlePresentationPolicy {
    private val canonicalUuid = Regex(
        "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
        RegexOption.IGNORE_CASE,
    )

    fun displayTitle(title: String?, conversationId: String?, fallback: String): String {
        val normalized = title.orEmpty().trim()
        return if (isRuntimePlaceholder(normalized, conversationId)) fallback else normalized
    }

    fun isRuntimePlaceholder(title: String, conversationId: String?): Boolean {
        val normalized = title.trim()
        if (normalized.isEmpty() || normalized in setOf("New conversation", "New Conversation", "新对话")) {
            return true
        }
        if (canonicalUuid.matches(normalized) || isLegacySessionPlaceholder(normalized)) {
            return true
        }
        val canonicalId = conversationId.orEmpty().trim()
        if (canonicalId.isEmpty()) return false
        return normalized == canonicalId || normalized == "Conversation ${canonicalId.take(8)}"
    }

    private fun isLegacySessionPlaceholder(value: String): Boolean {
        if (!value.startsWith("Session ")) return false
        val suffix = value.removePrefix("Session ")
        return suffix.length == 8 && suffix.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }
    }
}
