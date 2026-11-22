package com.nola.data.repository

import com.nola.domain.model.CustomDataSourceConfig
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidDataSourceReaderContentTypeTest {

    @Test
    fun malformedContentTypeFallsBackToUtf8WithoutCrashing() = runBlocking {
        val engine = MockEngine {
            respond(
                content = "Hello from source",
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, "not-a-valid-content-type;;;"),
            )
        }
        val reader = AndroidDataSourceReader(HttpClient(engine))
        val result = reader.read(
            CustomDataSourceConfig(
                id = "src-1",
                name = "Test Source",
                location = "https://example.test/feed",
            )
        )
        assertTrue(result.content.contains("Hello from source"))
    }
}
