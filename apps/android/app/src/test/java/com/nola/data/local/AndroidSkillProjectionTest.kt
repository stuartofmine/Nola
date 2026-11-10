package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidSkillProjectionTest {
    private fun snapshot(record: JsonObject, revision: Long = 1) = buildJsonObject {
        put("record", record); put("revision", revision)
    }
    private fun record(id: String = "review") = buildJsonObject {
        put("id", id); put("source", "user"); put("display_name", "Review")
        put("instructions_text", "Review text"); put("enabled", true)
        put("summary", "Summary"); put("version", "v1")
        put("required_tools", buildJsonArray {
            add(buildJsonObject { put("provider", "builtin"); put("tool_name", "read_file") })
        })
    }

    @Test
    fun userRecordsAreDecodedAndBuiltInsAreExplicitlyExcluded() {
        val builtin = JsonObject(record().toMutableMap().apply { put("source", JsonPrimitive("built_in")) })
        val records = decodeAndroidUserSkills(JsonArray(listOf(snapshot(record()), snapshot(builtin))))
        assertEquals(1, records.size)
        assertEquals("review", records.single().resourceId)
        assertEquals(listOf("read_file"), records.single().requiredTools)
        assertTrue(records.single().enabled)
        assertEquals(1L, records.single().canonicalRevision)
        assertEquals(record(), records.single().canonicalSkillRecord)
    }

    @Test
    fun invalidRecordCannotSilentlyDisappearFromAnOtherwiseValidList() {
        val malformed = listOf(
            JsonObject(record().toMutableMap().apply { remove("instructions_text") }),
            JsonObject(record().toMutableMap().apply { put("enabled", JsonPrimitive("invalid")) }),
            JsonObject(record().toMutableMap().apply { put("source", JsonPrimitive("unknown")) }),
            record(""),
            JsonObject(record().toMutableMap().apply { put("required_tools", JsonPrimitive("bad")) }),
        )
        for (invalid in malformed) {
            assertTrue("Expected the whole projection to fail: $invalid", runCatching {
                decodeAndroidUserSkills(JsonArray(listOf(snapshot(record("valid")), snapshot(invalid))))
            }.isFailure)
        }
    }

    @Test
    fun editorRequiresRevisionAndDoesNotSerializeItsCanonicalSnapshot() {
        for (invalid in listOf(record(), snapshot(record(), 0), snapshot(record(), -1))) {
            assertTrue(runCatching { decodeAndroidUserSkills(JsonArray(listOf(invalid))) }.isFailure)
        }
        val skill = decodeAndroidUserSkills(JsonArray(listOf(snapshot(record(), 7)))).single()
        val encoded = Json.encodeToJsonElement(com.nola.domain.model.UploadedContent.serializer(), skill).jsonObject
        assertFalse(encoded.containsKey("canonicalRevision"))
        assertFalse(encoded.containsKey("canonicalSkillRecord"))
    }
}
