package com.nola.domain.model

/** Shared conversation wire values. Null inherits the selected model's default. */
internal fun requireValidPromptResponseStyle(value: String?): String? {
    require(value == null || value in setOf("professional", "casual", "playful")) {
        "Unsupported conversation prompt response style"
    }
    return value
}
