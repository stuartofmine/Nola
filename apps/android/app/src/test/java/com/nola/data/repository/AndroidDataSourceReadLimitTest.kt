package com.nola.data.repository

import com.nola.domain.model.CustomDataSourceConfig
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidDataSourceReadLimitTest {
    @Test
    fun declaredOversizedResponseIsRejected() = runBlocking {
        val client = HttpClient(MockEngine {
            respond("small fixture", headers = headersOf("Content-Length", "100000000"))
        })
        try {
            val result = runCatching { AndroidDataSourceReader(client).read(source()) }
            assertTrue(result.exceptionOrNull()?.message?.contains("1 MB") == true)
        } finally { client.close() }
    }

    @Test
    fun responseWithoutContentLengthIsStillBounded() = runBlocking {
        val client = HttpClient(MockEngine { respond("x".repeat(1_000_001)) })
        try {
            val result = runCatching { AndroidDataSourceReader(client).read(source()) }
            assertTrue(result.exceptionOrNull()?.message?.contains("1 MB") == true)
        } finally { client.close() }
    }

    @Test
    fun inspectorCountsUtf8BytesRatherThanCharacters() {
        val text = "\u4e2d".repeat(400_000)
        val result = runCatching {
            DataSourceDocumentInspector.inspect("source", "https://example.test/data",
                "text/plain", text, 60)
        }
        assertTrue(result.exceptionOrNull()?.message?.contains("1 MB") == true)
    }

    private fun source() = CustomDataSourceConfig(
        id = "source", name = "Source", location = "https://example.test/data"
    )
}
