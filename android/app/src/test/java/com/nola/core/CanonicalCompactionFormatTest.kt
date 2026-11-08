package com.nola.core

import com.nola.domain.model.CompactedMemoryEntry
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.*
import org.junit.Test

class CanonicalCompactionFormatTest {
    @Test fun coreLineageUsesTheSameTimestampAndMessageIdentityFields() {
        val json = Json { ignoreUnknownKeys = true }
        val entry = json.decodeFromString<CompactedMemoryEntry>("""{
            "id":"entry", "createdAt":"2026-10-20T00:00:00Z", "summary":"observed",
            "rawExcerpt":"", "archivedMessageIDs":["m1"], "archiveID":"archive"
        }""")
        assertEquals(listOf("m1"), entry.archivedMessageIds)
        val encoded = json.parseToJsonElement(json.encodeToString(entry)).jsonObject
        assertTrue("archivedMessageIDs" in encoded)
        assertFalse("archivedMessageIds" in encoded)
        assertEquals(entry, json.decodeFromString<CompactedMemoryEntry>(encoded.toString()))
    }
    @Test fun nonCanonicalTimestampIsRejectedWithoutConversion() {
        assertTrue(runCatching { Json.decodeFromString<CompactedMemoryEntry>("""{
            "id":"entry","createdAt":1778000000000,"summary":"observed","rawExcerpt":"","archivedMessageIDs":["m1"]
        }""") }.isFailure)
    }
}
