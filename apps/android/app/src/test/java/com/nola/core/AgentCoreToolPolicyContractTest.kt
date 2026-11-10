package com.nola.core

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreToolPolicyContractTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun sharedCoreGoldenFixtureDecodesAsV2MinimalManifest() {
        val root = json.parseToJsonElement(sharedFixture().readText()).jsonObject
        val inputElement = root.getValue("input")
        val input = json.decodeFromString(
            AgentCoreToolPolicyInput.serializer(),
            inputElement.toString()
        )

        assertEquals(1, input.schemaVersion)
        assertEquals(1_000_000L, input.nowMs)
        assertEquals("ios-device-1", input.expectedIdentity.hostId)
        assertEquals(1L, input.runtime.manifestVersion)
        assertEquals(1_060_000L, input.runtime.expiresAtMs)
        assertEquals(2, input.runtime.resultEgressReceipts.size)
        assertEquals(listOf("device"), input.userPolicy.allowResultEgressRoutes)
        assertEquals(2, input.resultEgressGrants.size)
        assertTrue(input.hasValidResultEgressAuthorization("device_read_file", "device"))
        val allowedRuntimeToolKeys = setOf("tool_id", "handler_id", "schema", "route", "available")
        inputElement.jsonObject.getValue("runtime").jsonObject.getValue("tools")
            .jsonArray.forEach { tool ->
                assertEquals(allowedRuntimeToolKeys, tool.jsonObject.keys)
                assertFalse("capability" in tool.jsonObject)
                assertFalse("risk" in tool.jsonObject)
                assertFalse("permission" in tool.jsonObject)
                assertFalse("requires_result_egress" in tool.jsonObject)
            }
    }

    @Test
    fun sharedGoldenReceiptFieldsDecodeThroughAndroidCodec() {
        val root = json.parseToJsonElement(sharedFixture().readText()).jsonObject
        val expectedReceipt = root.getValue("expected").jsonObject
            .getValue("receipt").jsonObject
        val enabled = expectedReceipt.getValue("enabled_tool").jsonObject
        val inputTool = root.getValue("input").jsonObject.getValue("runtime").jsonObject
            .getValue("tools").jsonArray.first().jsonObject
        val output = buildJsonObject {
            put("schema_version", expectedReceipt.getValue("schema_version"))
            put("platform", expectedReceipt.getValue("platform"))
            put("policy_digest", JsonPrimitive("a".repeat(64)))
            put("manifest_digest", JsonPrimitive("b".repeat(64)))
            put("manifest_valid_until_ms", expectedReceipt.getValue("manifest_valid_until_ms"))
            put("enabled_tools", JsonArray(listOf(JsonObject(enabled + mapOf(
                "enabled" to JsonPrimitive(true),
                "schema" to inputTool.getValue("schema")
            )))))
            put("disabled_tools", JsonArray(emptyList()))
        }

        val decoded = AgentCoreToolPolicyCodec.decode(output.toString().encodeToByteArray())
        requireNotNull(decoded)
        assertEquals(1, decoded.schemaVersion)
        assertEquals("ios", decoded.platform)
        assertEquals(64, decoded.policyDigest.length)
        assertEquals("device_read_file", decoded.enabledTools.single().toolId)
        assertEquals("scoped_resource", decoded.enabledTools.single().permission)
        assertTrue(decoded.enabledTools.single().requiresResultEgress)
    }

    private fun sharedFixture(): File {
        val workingDirectory = System.getProperty("user.dir") ?: "."
        return generateSequence(File(workingDirectory).canonicalFile) { it.parentFile }
            .map { root -> File(root, "core/fixtures/tool_policy_v1_golden.json") }
            .firstOrNull(File::isFile)
            ?: error("Shared Core tool policy fixture is missing")
    }
}
