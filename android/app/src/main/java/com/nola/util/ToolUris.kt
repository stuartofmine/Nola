package com.nola.util

import java.net.URI

internal fun httpToolUriOrNull(raw: String?): String? {
    val value = raw?.trim().orEmpty()
    if (value.isEmpty() || hasUnsafeToolUriCharacters(value)) return null

    val uri = runCatching { URI(value) }.getOrNull() ?: return null
    val scheme = uri.scheme?.lowercase()
    if (scheme != "http" && scheme != "https") return null
    if (uri.host.isNullOrBlank() || uri.userInfo != null) return null

    return value
}

internal fun rootRelativeToolPathOrNull(raw: String?): String? {
    val value = raw?.trim().orEmpty()
    if (value.isEmpty() || hasUnsafeToolUriCharacters(value)) return null
    if (!value.startsWith("/") || value.startsWith("//")) return null
    return value
}

private fun hasUnsafeToolUriCharacters(value: String): Boolean {
    return value.any { it <= ' ' || it.code == 0x7F }
}
