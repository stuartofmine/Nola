package com.nola.data.repository

import com.nola.domain.model.StructuredMemoryRecord
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ContextRepositoryMemoryMappingTest {
    @Test
    fun persistentRetentionDoesNotLeakExpiryIntoTaggedPayload() {
        val retention = StructuredMemoryRecord(
            id = "memory-1",
            owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
            content = "Remember this",
            retention = "persistent",
            expiresAt = 1_700_000_000_000,
        ).toAgentCoreMemoryRetention()

        assertEquals("permanent", retention?.mode)
        assertNull(retention?.expiresAt)
    }

    @Test
    fun expiringRetentionRequiresAndMapsTimestamp() {
        val retention = StructuredMemoryRecord(
            id = "memory-2",
            owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
            content = "Temporary memory",
            retention = "expires_at",
            expiresAt = 1_700_000_000_000,
        ).toAgentCoreMemoryRetention()

        assertEquals("expires_at", retention?.mode)
        assertEquals("2023-11-14T22:13:20Z", retention?.expiresAt)
    }

    @Test
    fun malformedRetentionIsExcludedBeforeCallingCore() {
        assertNull(
            StructuredMemoryRecord(
                id = "memory-3",
                owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
                content = "Missing expiry",
                retention = "expires_at",
            ).toAgentCoreMemoryRetention(),
        )
        assertNull(
            StructuredMemoryRecord(
                id = "memory-4",
                owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
                content = "Unknown retention",
                retention = "forever-ish",
            ).toAgentCoreMemoryRetention(),
        )
    }
}
