@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import com.nola.domain.model.StructuredMemoryRecord
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AgentCoreMemorySensitivityContractTest {
    private val json = Json {
        encodeDefaults = true
        explicitNulls = false
    }

    @Test
    fun policySerializesTheTypedSensitivityAllowlistForCore() {
        val policy = AgentCoreMemoryPolicy(
            longTermEnabled = true,
            automaticExtractionEnabled = true,
            searchLimit = 20,
            minimumConfidence = 0.5f,
            allowedSensitivities = listOf("health.private"),
        )

        val encoded = json.parseToJsonElement(json.encodeToString(policy)).jsonObject
        assertEquals(
            "health.private",
            encoded.getValue("allowed_sensitivities").jsonArray.single().jsonPrimitive.content,
        )
    }

    @Test
    fun storedMemoryRoundTripPreservesOwnerAndSensitivity() {
        val memory = StructuredMemoryRecord(
            id = "memory-sensitive",
            owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
            content = "typed protected fact",
            sensitivity = "health.private",
        )

        val restored = json.decodeFromString<StructuredMemoryRecord>(json.encodeToString(memory))
        assertEquals("health.private", restored.sensitivity)

        assertEquals(memory.owner, restored.owner)
        val ordinary = json.decodeFromString<StructuredMemoryRecord>(
            """{"id":"ordinary","owner":{"kind":"profile","profile_id":"local"},"content":"ordinary fact"}""",
        )
        assertNull(ordinary.sensitivity)
    }

    @Test(expected = kotlinx.serialization.MissingFieldException::class)
    fun missingOwnerIsRejectedInsteadOfInferred() {
        json.decodeFromString<StructuredMemoryRecord>("""{"id":"unbound","content":"ordinary fact"}""")
    }
}
