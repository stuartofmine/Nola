package com.nola.data.remote

import io.ktor.client.HttpClient
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.pluginOrNull
import io.ktor.client.plugins.timeout
import io.ktor.client.request.HttpRequestBuilder

/** Active streams have an idle bound, not the ordinary JSON request's total deadline. */
internal fun HttpRequestBuilder.applyStreamingTimeouts(client: HttpClient) {
    if (client.pluginOrNull(HttpTimeout) != null) timeout {
        requestTimeoutMillis = HttpTimeout.INFINITE_TIMEOUT_MS
        socketTimeoutMillis = 300_000
    }
}
