package com.nola.domain.model

import java.net.URI

/** Shared admission rule for remote model endpoints across UI and persistence. */
object RemoteModelEndpointURL {
    fun isValid(value: String): Boolean = runCatching {
        val uri = URI(value.trim())
        (uri.scheme.equals("http", ignoreCase = true) || uri.scheme.equals("https", ignoreCase = true)) &&
            !uri.host.isNullOrEmpty() && uri.rawUserInfo == null && uri.rawFragment == null
    }.getOrDefault(false)
}
