package com.nola.data.remote

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsText
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

class StreamingHttpPolicyTest {
    @Test fun streamOutlivesOrdinaryRequestDeadline() = runBlocking {
        val client = HttpClient(MockEngine {
            delay(150)
            respond("data: [DONE]\n\n")
        }) { install(HttpTimeout) { requestTimeoutMillis = 25 } }
        try {
            assertEquals("data: [DONE]\n\n", client.get("https://fixture.invalid/stream") {
                applyStreamingTimeouts(client)
            }.bodyAsText())
        } finally { client.close() }
    }
}
