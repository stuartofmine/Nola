package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidAutomationExecutionTest {
    @Test fun executionDefinitionRoundtripsExplicitAuthority() {
        val spec = AndroidAutomationSpec("Inspect source", "android-device:fixture", "cautious",
            listOf("web_source" to "source"), listOf("skill"))
        assertEquals(spec, AndroidAutomationSpec.decode(spec.toJson()))
        val row = buildJsonObject {
            put("id", "definition"); put("revision", 17)
            put("metadata_json", "{}")
            put("execution_spec", buildJsonObject { put("kind", "agent"); put("spec", spec.toJson()) })
        }
        assertEquals(17L, decodeAndroidAutomationDefinition(row).revision)
        assertTrue(runCatching { decodeAndroidAutomationDefinition(JsonObject(row - "revision")) }.isFailure)
        assertTrue(runCatching { decodeAndroidAutomationDefinition(JsonObject(row + ("revision" to JsonPrimitive(0)))) }.isFailure)
    }

    @Test fun onlyCurrentPermissionProfilesCanOwnAnAutomationDefinition() {
        for (profile in listOf("cautious", "allow_all")) {
            val spec = AndroidAutomationSpec("Inspect source", "android-device:fixture", profile,
                emptyList(), emptyList())
            assertEquals(profile, AndroidAutomationSpec.decode(spec.toJson()).permissionProfile)
        }
        for (invalid in listOf("safe", "recommended", "allow-all", " cautious ", "", "unknown")) {
            assertThrows(IllegalArgumentException::class.java) {
                AndroidAutomationSpec("Inspect source", "android-device:fixture", invalid, emptyList(), emptyList())
            }
        }
    }

    @Test fun occurrenceLeaseCarriesExactDefinitionAndOwner() {
        val lease = AndroidAutomationLease("occurrence", 23, "worker", "lease").toJson()
        assertEquals(23, lease.getValue("definition_revision").jsonPrimitive.int)
        assertEquals("occurrence", lease.getValue("occurrence_id").jsonPrimitive.content)
        assertEquals("worker", lease.getValue("worker_id").jsonPrimitive.content)
        assertTrue(runCatching { AndroidAutomationLease("occurrence", 0, "worker", "lease") }.isFailure)
    }
}
