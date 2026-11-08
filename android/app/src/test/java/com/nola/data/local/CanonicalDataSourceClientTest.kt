package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class CanonicalDataSourceClientTest {
    @Test fun oversizedContentIsRejectedBeforeCreatingOrInvalidatingSource() {
        val commands = mutableListOf<JsonObject>()
        val directory = java.nio.file.Files.createTempDirectory("source-size-test").toFile()
        try {
            val adapter = AndroidCanonicalDataSources(
                CanonicalDataSourceClient { commands += it; JsonNull }, directory
            )
            val source = com.nola.domain.model.CustomDataSourceConfig(
                id = "large", name = "Large", location = "https://example.test/data",
                cachedContent = "x".repeat(8 * 1024 * 1024 + 1),
                validationContract = com.nola.domain.model.DataSourceValidationContract(
                    sourceId = "large", readerDescription = "HTTP", format = "text"
                )
            )
            assertTrue(runCatching { adapter.save(source) }.exceptionOrNull() is IllegalArgumentException)
            assertEquals(listOf("get"), commands.map { it["command"]!!.jsonObject["type"]!!.jsonPrimitive.content })
        } finally { directory.deleteRecursively() }
    }

    @Test fun deleteCarriesExplicitRevisionWithoutReadingOrRewritingOtherSources() {
        val commands = mutableListOf<JsonObject>()
        val client = CanonicalDataSourceClient { commands += it; JsonPrimitive(true) }
        client.delete("source-1", 27, 100)
        assertEquals(1, commands.size)
        val command = commands.single()["command"]!!.jsonObject
        assertEquals("delete", command["type"]!!.jsonPrimitive.content)
        assertEquals(27, command["expected_revision"]!!.jsonPrimitive.int)
        assertEquals("source-1", command["id"]!!.jsonPrimitive.content)
        assertFalse(command.containsKey("input"))
    }

    @Test fun missingSourceIsNullButMalformedListIsNotSilentlyDiscarded() {
        assertNull(CanonicalDataSourceClient { JsonNull }.get("missing"))
        val malformed = CanonicalDataSourceClient { JsonArray(listOf(buildJsonObject { put("id", "broken") })) }
        assertTrue(runCatching { malformed.list() }.isFailure)
    }
}
