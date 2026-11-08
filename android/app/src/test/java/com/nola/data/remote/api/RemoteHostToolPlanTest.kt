package com.nola.data.remote.api

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class RemoteHostToolPlanTest {
    private fun fixture(): JsonObject = Json.parseToJsonElement("""{
      "source_host_id":"linux-host","result":{"schema_version":1,
      "reference":{"manifest_id":"issued","host_id":"linux-host","client_id":"android","session_id":"conversation","catalog_revision":"${"a".repeat(64)}"},
      "issued_at_ms":1000,"expires_at_ms":3000,
      "tools":[{"tool_id":"custom_mcp","route":"agent_host","schema":{"type":"function","function":{"name":"custom_mcp","parameters":{"type":"object"}}}}]}}
    """).jsonObject

    @Test fun usesExecutionHostReferenceWithoutComparingIndependentDeviceClock() {
        val value = fixture()
        assertEquals(value["result"]!!.jsonObject["reference"], validatedHostToolPlanReference(value, "linux-host", "android", "conversation"))
        for ((host, client, session) in listOf(Triple("other", "android", "conversation"), Triple("linux-host", "other", "conversation"), Triple("linux-host", "android", "other"))) {
            assertThrows(IllegalArgumentException::class.java) { validatedHostToolPlanReference(value, host, client, session) }
        }
        assertEquals(value["result"]!!.jsonObject["reference"], validatedHostToolPlanReference(value, "linux-host", "android", "conversation"))
    }

    @Test fun acceptsHostClockIntervalsOnEitherSideOfDeviceTime() {
        for (issued in listOf(1L, System.currentTimeMillis() + 86_400_000L)) {
            val value = fixture()
            val result = JsonObject(value.getValue("result").jsonObject + mapOf(
                "issued_at_ms" to JsonPrimitive(issued), "expires_at_ms" to JsonPrimitive(issued + 300_000L)))
            assertEquals(result["reference"], validatedHostToolPlanReference(
                JsonObject(value + ("result" to result)), "linux-host", "android", "conversation"))
        }
    }

    @Test fun rejectsDuplicatedToolsAndWrongSchemaIdentity() {
        val value = fixture()
        val result = value["result"]!!.jsonObject
        val tool = result["tools"]!!.jsonArray.first()
        val duplicated = JsonObject(value + ("result" to JsonObject(result + ("tools" to JsonArray(listOf(tool, tool))))))
        assertThrows(IllegalArgumentException::class.java) { validatedHostToolPlanReference(duplicated, "linux-host", "android", "conversation") }
        val wrong = Json.parseToJsonElement(value.toString().replace("\"name\":\"custom_mcp\"", "\"name\":\"other\"")).jsonObject
        assertThrows(IllegalArgumentException::class.java) { validatedHostToolPlanReference(wrong, "linux-host", "android", "conversation") }
    }

    @Test fun rejectsCoercedNumbersBlankIdentitiesAndInvalidTimeBounds() {
        val value = fixture()
        val result = value["result"]!!.jsonObject
        for ((key, replacement) in listOf(
            "legacy" to JsonPrimitive(true), "schema_version" to JsonPrimitive("1"), "schema_version" to JsonPrimitive(true),
            "issued_at_ms" to JsonPrimitive("1000"), "expires_at_ms" to JsonPrimitive("3000"),
            "issued_at_ms" to JsonPrimitive(0), "issued_at_ms" to JsonPrimitive(3000),
            "expires_at_ms" to JsonPrimitive(1000), "expires_at_ms" to JsonPrimitive(301001),
            "expires_at_ms" to JsonPrimitive(9_007_199_254_740_992L))) {
            val changed = JsonObject(value + ("result" to JsonObject(result + (key to replacement))))
            assertThrows(RuntimeException::class.java) { validatedHostToolPlanReference(changed, "linux-host", "android", "conversation") }
        }
        val ref = result["reference"]!!.jsonObject
        for (replacement in listOf(JsonPrimitive(" \n"), JsonPrimitive("id\u0000suffix"), JsonPrimitive("id\u0085suffix"), JsonPrimitive(123), JsonPrimitive("x".repeat(513)))) {
            val changed = JsonObject(value + ("result" to JsonObject(result + ("reference" to JsonObject(ref + ("manifest_id" to replacement))))))
            assertThrows(RuntimeException::class.java) { validatedHostToolPlanReference(changed, "linux-host", "android", "conversation") }
        }
        val numericHost = JsonObject(value + ("source_host_id" to JsonPrimitive(123)))
        assertThrows(RuntimeException::class.java) { validatedHostToolPlanReference(numericHost, "123", "android", "conversation") }
        val unknownReference = JsonObject(value + ("result" to JsonObject(result + ("reference" to JsonObject(ref + ("legacy" to JsonPrimitive(true)))))))
        assertThrows(RuntimeException::class.java) { validatedHostToolPlanReference(unknownReference, "linux-host", "android", "conversation") }
        val tool = result["tools"]!!.jsonArray.first().jsonObject
        val malformedTools = JsonArray(listOf(JsonObject(tool + ("legacy" to JsonPrimitive(true)))))
        val unknownTool = JsonObject(value + ("result" to JsonObject(result + ("tools" to malformedTools))))
        assertThrows(RuntimeException::class.java) { validatedHostToolPlanReference(unknownTool, "linux-host", "android", "conversation") }
    }
}
