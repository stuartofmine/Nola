package com.nola.ui.screen.chat

import java.net.URI
import java.util.Locale

/** A model-visible source descriptor grants no access to device files/providers. */
internal fun isExternalAnswerSource(uri: String): Boolean = runCatching {
    val target = URI(uri)
    target.scheme?.lowercase(Locale.ROOT) in setOf("http", "https") &&
        !target.host.isNullOrBlank() && target.rawUserInfo == null
}.getOrDefault(false)
