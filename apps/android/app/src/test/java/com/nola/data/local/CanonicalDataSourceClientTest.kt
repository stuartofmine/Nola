package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class CanonicalDataSourceClientTest {
    @Test fun validatedInputFormatPreservesCustomOutputSchemaAndDoesNotGuessFromOldSchemas() {
        val original = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject { put("items", buildJsonObject { put("type", "array") }) })
            put("x-custom-reader", "structured")
        }
        assertEquals("", readSourceFormat(original))
        val contract = com.nola.domain.model.DataSourceValidationContract(
            sourceId = "source", readerDescription = "Custom reader", format = "JSON",
            readRecipe = com.nola.domain.model.DataSourceReadRecipe(
                method = com.nola.domain.model.DataSourceReadMethod.HttpGet, operationId = "read",
                outputSchemaJson = original.toString(),
            )
        )
        val annotated = contract.toCanonicalContract().getValue("readRecipe").jsonObject.getValue("outputSchema").jsonObject
        assertEquals(original, JsonObject(annotated - "x-nola-source-format"))
        assertEquals("JSON", readSourceFormat(annotated))
        val existing = JsonObject(original + ("x-nola-source-format" to JsonPrimitive("NDJSON")))
        val custom = contract.copy(readRecipe = contract.readRecipe!!.copy(outputSchemaJson = existing.toString()))
        assertEquals(existing, custom.toCanonicalContract().getValue("readRecipe").jsonObject.getValue("outputSchema"))
        assertEquals("", readSourceFormat(JsonObject(original + ("x-nola-source-format" to JsonNull))))
    }

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

    @Test fun v2DaemonEnvelopesCreateRenameReadAndDeleteDataSource() {
        val records = linkedMapOf<String, JsonObject>()
        val directory = java.nio.file.Files.createTempDirectory("source-v2-roundtrip").toFile()
        try {
            val client = CanonicalDataSourceClient { envelope ->
                val cmd = envelope.getValue("command").jsonObject
                when (cmd.getValue("type").jsonPrimitive.content) {
                    "list" -> buildJsonObject {
                        put("kind", "data_source")
                        put("items", JsonArray(records.values.toList()))
                    }
                    "get" -> records[cmd.getValue("id").jsonPrimitive.content] ?: JsonNull
                    "create_draft" -> {
                        val input = cmd.getValue("input").jsonObject
                        val id = input.getValue("id").jsonPrimitive.content
                        records[id] = input
                        buildJsonObject { put("ok", true); put("id", id); put("record", input) }
                    }
                    "update_configuration" -> {
                        val input = cmd.getValue("input").jsonObject
                        // Mirrors v2 daemon.rs precedence: input["configuration"] takes priority over input["record"].
                        val rec = (input["configuration"] ?: input["record"] ?: input).jsonObject
                        val id = cmd.getValue("id").jsonPrimitive.content
                        val merged = JsonObject(records.getValue(id) + rec)
                        records[id] = merged
                        buildJsonObject { put("ok", true); put("id", id); put("record", merged) }
                    }
                    "request_validation" -> buildJsonObject {
                        put("ok", true)
                        put("id", cmd.getValue("id").jsonPrimitive.content)
                        put("status", "pending")
                    }
                    "record_validation" -> {
                        val id = cmd.getValue("id").jsonPrimitive.content
                        records[id] = JsonObject(records.getValue(id) + ("validation" to cmd))
                        buildJsonObject { put("ok", true); put("id", id); put("validation", cmd) }
                    }
                    "request_refresh" -> buildJsonObject {
                        put("ok", true)
                        put("id", cmd.getValue("id").jsonPrimitive.content)
                        put("status", "refresh_requested")
                    }
                    "record_read" -> buildJsonObject {
                        put("ok", true)
                        put("id", cmd.getValue("id").jsonPrimitive.content)
                        put("recorded_at_ms", 1000L)
                    }
                    "set_default_enabled" -> {
                        val id = cmd.getValue("id").jsonPrimitive.content
                        val enabled = cmd.getValue("enabled").jsonPrimitive.boolean
                        val updated = JsonObject(records.getValue(id) + ("default_enabled" to JsonPrimitive(enabled)))
                        records[id] = updated
                        buildJsonObject { put("ok", true); put("id", id); put("default_enabled", enabled); put("record", updated) }
                    }
                    "delete" -> {
                        val id = cmd.getValue("id").jsonPrimitive.content
                        val deleted = records.remove(id) != null
                        buildJsonObject { put("ok", true); put("deleted", deleted); put("id", id) }
                    }
                    else -> error("Unexpected command: $cmd")
                }
            }
            val adapter = AndroidCanonicalDataSources(client, directory)
            val contract = com.nola.domain.model.DataSourceValidationContract(
                sourceId = "src-1",
                readerDescription = "HTTP GET -> JSON -> RawEvent",
                format = "JSON",
                schemaFields = listOf("reading"),
                redactedSample = """{"reading": 25}""",
                recordCount = 1,
                maximumAgeMinutes = 60,
                validationState = com.nola.domain.model.DataSourceValidationState.Verified,
                validatedAt = 1_000L,
                observedAt = 1_000L,
                readRecipe = com.nola.domain.model.DataSourceReadRecipe(
                    method = com.nola.domain.model.DataSourceReadMethod.HttpGet,
                    operationId = "android.http.get",
                    configurationKeys = listOf("url", "maximumBytes"),
                    outputSchemaJson = """{"type":"object"}""",
                ),
            )
            val created = com.nola.domain.model.CustomDataSourceConfig(
                id = "src-1",
                name = "Audit source",
                location = "http://127.0.0.1:9999/fixture",
                cachedContent = """{"reading": 25}""",
                validationContract = contract,
            )
            adapter.save(created)
            val listed = adapter.list()
            assertEquals(1, listed.size)
            assertEquals("Audit source", listed.single().name)
            assertEquals("""{"reading": 25}""", listed.single().cachedContent)
            assertNotNull(listed.single().validationContract)

            adapter.save(listed.single().copy(name = "Audit source renamed"))
            val renamed = adapter.list().single()
            assertEquals("Audit source renamed", renamed.name)

            adapter.delete("src-1")
            assertTrue(adapter.list().isEmpty())
        } finally {
            directory.deleteRecursively()
        }
    }
}
