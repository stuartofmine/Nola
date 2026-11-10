package com.nola.util

private val hostToolSensitivePatterns = listOf(
    Regex("client[_\\s-]?secret", RegexOption.IGNORE_CASE),
    Regex("api[_\\s-]?key", RegexOption.IGNORE_CASE),
    Regex("access[_\\s-]?token", RegexOption.IGNORE_CASE),
    Regex("refresh[_\\s-]?token", RegexOption.IGNORE_CASE),
    Regex("id[_\\s-]?token", RegexOption.IGNORE_CASE),
    Regex("\"?(?:token|secret|cookie|session(?:[_\\s-]?id)?|auth)\"?\\s*[:=]", RegexOption.IGNORE_CASE),
    Regex("authorization", RegexOption.IGNORE_CASE),
    Regex("password", RegexOption.IGNORE_CASE),
    Regex("private[_\\s-]?key", RegexOption.IGNORE_CASE),
    Regex("credential", RegexOption.IGNORE_CASE),
    Regex("\\bBearer\\s+[A-Za-z0-9._~+/=-]{8,}", RegexOption.IGNORE_CASE),
    Regex("sk-[A-Za-z0-9_-]{16,}"),
    Regex("GOCSPX-[A-Za-z0-9_-]+"),
    Regex("gh[pousr]_[A-Za-z0-9_]{20,}"),
    Regex("github_pat_[A-Za-z0-9_]{20,}"),
    Regex("-----BEGIN [A-Z ]*PRIVATE KEY-----")
)

internal fun hostToolContainsSensitiveData(vararg values: String?): Boolean {
    val text = values.filterNotNull().joinToString(separator = "\n")
    return hostToolSensitivePatterns.any { it.containsMatchIn(text) }
}

internal fun redactHostToolText(value: String?): String {
    var text = value.orEmpty()
    val replacements = listOf(
        Regex("(?is)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----") to "[hidden sensitive key]",
        Regex("(?i)(\"?(?:client[_\\s-]?secret|api[_\\s-]?key|access[_\\s-]?token|refresh[_\\s-]?token|id[_\\s-]?token|token|secret|cookie|session(?:[_\\s-]?id)?|auth|authorization|password|private[_\\s-]?key|credential)\"?\\s*(?::|=|\\bis\\b)\\s*\"?)((?:Bearer\\s+)?[^\"\\s,}]+)(\"?)") to "\$1[hidden]\$3",
        Regex("\\bBearer\\s+[A-Za-z0-9._~+/=-]{8,}", RegexOption.IGNORE_CASE) to "Bearer [hidden]",
        Regex("sk-[A-Za-z0-9_-]{16,}") to "sk-[hidden]",
        Regex("GOCSPX-[A-Za-z0-9_-]+") to "GOCSPX-[hidden]",
        Regex("gh[pousr]_[A-Za-z0-9_]{20,}") to "gh[hidden]",
        Regex("github_pat_[A-Za-z0-9_]{20,}") to "github_pat_[hidden]"
    )
    replacements.forEach { (pattern, replacement) ->
        text = pattern.replace(text, replacement)
    }
    return text
}
