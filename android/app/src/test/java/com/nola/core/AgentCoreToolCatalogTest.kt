package com.nola.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreToolCatalogTest {
    @Test
    fun fileResultsExposeCoreArtifacts() {
        val artifacts = androidAgentFileArtifacts(
            "content://com.nola.documents/reports/report.docx",
            "create"
        )

        assertEquals(1, artifacts.size)
        assertEquals("report.docx", artifacts[0].jsonObject["name"]?.jsonPrimitive?.content)
        assertEquals("create", artifacts[0].jsonObject["operation"]?.jsonPrimitive?.content)
    }

    @Test
    fun registryContainsOnlyCanonicalV2HandlerFacts() {
        assertEquals(
            listOf(
                "device_read_file",
                "device_write_file",
                "device_clipboard_read",
                "device_clipboard_write",
                "device_get_location",
                "device_search_calendar_events",
                "device_create_calendar_event",
                "memory_search",
                "skill_invoke"
            ),
            AndroidAgentToolRegistry.candidates.map { it.toolId }
        )
        assertTrue(AndroidAgentToolRegistry.candidates.all {
            AndroidAgentToolRegistry.canHandle(it.toolId, it.handlerId)
        })
        val expectedRisk = mapOf(
            "device_read_file" to "ordinary_read", "device_write_file" to "unknown",
            "device_clipboard_read" to "sensitive_read", "device_clipboard_write" to "overwrite",
            "device_get_location" to "sensitive_read", "device_search_calendar_events" to "sensitive_read",
            "device_create_calendar_event" to "new_file", "memory_search" to "ordinary_read",
            "skill_invoke" to "ordinary_read",
        )
        for (candidate in AndroidAgentToolRegistry.candidates) {
            val metadata = (candidate.schema as JsonObject).getValue("x_nola").jsonObject
            assertEquals(setOf("permission_contract"), metadata.keys)
            val contract = metadata.getValue("permission_contract").jsonObject
            assertEquals(setOf("risk"), contract.keys)
            assertEquals(expectedRisk.getValue(candidate.toolId), contract.getValue("risk").jsonPrimitive.content)
        }
    }

    @Test
    fun missingCoreResolutionFailsClosed() {
        val identity = identity()
        val plan = AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.DeviceLocal,
            snapshot = snapshot(),
            handlerCapabilities = AndroidAgentToolRegistry.capabilities,
            handlerIds = AndroidAgentToolRegistry.handlerIds,
            identity = identity,
            resultEgressAuthorization = authorization(identity),
            resolver = AgentCoreToolPolicyResolver { null }
        )

        assertTrue(plan.tools.isEmpty())
        assertFalse(plan.receiptIsValid())
    }

    @Test
    fun exactV2ReceiptProducesCoreOwnedRuntimeMetadataAndTripleEgressAuthorization() {
        val selected = AndroidAgentToolRegistry.candidates.first()
        val identity = identity()
        val plan = AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.Hybrid,
            snapshot = snapshot(),
            handlerCapabilities = AndroidAgentToolRegistry.capabilities,
            handlerIds = AndroidAgentToolRegistry.handlerIds,
            identity = identity,
            resultEgressAuthorization = authorization(identity),
            resolver = AgentCoreToolPolicyResolver { input ->
                assertEquals(1, input.schemaVersion)
                assertEquals(identity, input.expectedIdentity)
                assertEquals(identity.sessionId, input.runtime.sessionId)
                assertEquals(listOf("agent_host", "device"), input.runtime.routes)
                assertTrue(input.runtime.tools.all { it.available })
                assertEquals(input.runtime.tools.size, input.runtime.resultEgressReceipts.size)
                assertEquals(listOf("device"), input.userPolicy.allowResultEgressRoutes)
                assertEquals(input.runtime.tools.size, input.resultEgressGrants.size)
                assertTrue(input.hasValidResultEgressAuthorization(selected.toolId, "device"))
                output(input, selected, capability = "files_read")
            }
        )

        assertTrue(plan.receiptIsValid())
        assertTrue(plan.allows(selected.toolId, "files_read"))
        assertEquals("a".repeat(64), plan.policyDigest)
        assertEquals("b".repeat(64), plan.manifestDigest)
        val runtimeMetadata = (plan.tools.single() as JsonObject)["x_nola"] as JsonObject
        assertEquals("files_read", runtimeMetadata["capability"].toString().trim('"'))
        assertEquals("scoped_resource", runtimeMetadata["authorization"].toString().trim('"'))
        assertEquals(plan.policyDigest, runtimeMetadata["policy_digest"].toString().trim('"'))
    }

    @Test
    fun typedInputRejectsMissingTripleAndIdentitySessionOrExpiryMismatch() {
        val selected = AndroidAgentToolRegistry.candidates.first()
        val identity = identity()
        lateinit var input: AgentCoreToolPolicyInput
        AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.DeviceLocal,
            snapshot = snapshot(),
            handlerCapabilities = AndroidAgentToolRegistry.capabilities,
            handlerIds = AndroidAgentToolRegistry.handlerIds,
            identity = identity,
            resultEgressAuthorization = authorization(identity),
            resolver = AgentCoreToolPolicyResolver { actual ->
                input = actual
                output(actual, selected, capability = "files_read")
            }
        )
        val receipt = input.runtime.resultEgressReceipts.first { it.toolId == selected.toolId }
        val grant = input.resultEgressGrants.first { it.toolId == selected.toolId }

        assertTrue(input.hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(runtime = input.runtime.copy(resultEgressReceipts = emptyList()))
            .hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(userPolicy = input.userPolicy.copy(allowResultEgressRoutes = emptyList()))
            .hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(resultEgressGrants = emptyList())
            .hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(expectedIdentity = identity.copy(hostId = "other-device"))
            .hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(runtime = input.runtime.copy(
            resultEgressReceipts = input.runtime.resultEgressReceipts.map {
                if (it.toolId == selected.toolId) it.copy(sessionId = "other-session") else it
            }
        )).hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(runtime = input.runtime.copy(
            resultEgressReceipts = input.runtime.resultEgressReceipts.map {
                if (it.toolId == selected.toolId) {
                    it.copy(issuedAtMs = input.nowMs - 2, expiresAtMs = input.nowMs - 1)
                } else it
            }
        )).hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(resultEgressGrants = input.resultEgressGrants.map {
            if (it.toolId == selected.toolId) it.copy(clientId = "other-client") else it
        }).hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(resultEgressGrants = input.resultEgressGrants.map {
            if (it.toolId == selected.toolId) it.copy(sessionId = "other-session") else it
        }).hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertFalse(input.copy(resultEgressGrants = input.resultEgressGrants.map {
            if (it.toolId == selected.toolId) it.copy(expiresAtMs = receipt.expiresAtMs + 1) else it
        }).hasValidResultEgressAuthorization(selected.toolId, "device"))
        assertEquals(receipt.expiresAtMs, grant.expiresAtMs)
    }

    @Test
    fun mismatchedOrExpiredTurnAuthorizationFailsClosed() {
        val selected = AndroidAgentToolRegistry.candidates.first()
        val identity = identity()
        fun plan(authorization: AndroidAgentResultEgressAuthorization) =
            AndroidAgentToolPolicyPlanner.plan(
                executionMode = AndroidAgentExecutionMode.DeviceLocal,
                snapshot = snapshot(),
                handlerCapabilities = AndroidAgentToolRegistry.capabilities,
                handlerIds = AndroidAgentToolRegistry.handlerIds,
                identity = identity,
                resultEgressAuthorization = authorization,
                resolver = AgentCoreToolPolicyResolver { input ->
                    output(input, selected, capability = "files_read")
                }
            )

        val nowMs = System.currentTimeMillis()
        assertTrue(plan(authorization(identity.copy(sessionId = "other-session"), nowMs)).tools.isEmpty())
        assertTrue(plan(AndroidAgentResultEgressAuthorization(
            route = "device",
            identity = identity,
            issuedAtMs = nowMs - 2,
            expiresAtMs = nowMs - 1
        )).tools.isEmpty())
    }

    @Test
    fun backgroundOrMissingHandlerFactsCannotBeReenabledByCore() {
        val selected = AndroidAgentToolRegistry.candidates.first()
        val identity = identity()
        val background = AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.DeviceLocal,
            snapshot = snapshot(isForeground = false),
            handlerCapabilities = AndroidAgentToolRegistry.capabilities,
            handlerIds = AndroidAgentToolRegistry.handlerIds,
            identity = identity,
            resultEgressAuthorization = authorization(identity),
            resolver = AgentCoreToolPolicyResolver { input ->
                assertTrue(input.runtime.tools.none { it.available })
                output(input, selected, capability = "files_read")
            }
        )
        val missingHandler = AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.DeviceLocal,
            snapshot = snapshot(),
            handlerCapabilities = AndroidAgentToolRegistry.capabilities,
            handlerIds = emptySet(),
            identity = identity,
            resultEgressAuthorization = authorization(identity),
            resolver = AgentCoreToolPolicyResolver { input ->
                assertTrue(input.runtime.tools.none { it.available })
                output(input, selected, capability = "files_read")
            }
        )

        assertTrue(background.tools.isEmpty())
        assertTrue(missingHandler.tools.isEmpty())
    }

    @Test
    fun receiptWhitelistBindsCoreMetadataAndExpires() {
        val nowMs = System.currentTimeMillis()
        val plan = AgentCoreAndroidToolPlan(
            tools = emptyList(),
            grantedCapabilities = listOf("files_read"),
            allowedTools = listOf(
                AgentCoreAllowedAndroidTool(
                    toolId = "device_read_file",
                    handlerId = "android.files.read",
                    capability = "files_read",
                    route = "device",
                    permission = "scoped_resource",
                    authorization = "granted",
                    risk = "read_only",
                    requiresResultEgress = true
                )
            ),
            policyDigest = "a".repeat(64),
            manifestDigest = "b".repeat(64),
            manifestValidUntilMs = nowMs + 1_000,
            manifestId = "manifest",
            manifestVersion = 1
        )

        assertTrue(plan.allows("device_read_file", "files_read", "device"))
        assertFalse(plan.allows("device_read_file", "network", "device"))
        assertFalse(plan.allows("device_read_file", "files_read", "agent_host"))
        assertFalse(plan.receiptIsValid(nowMs + 2_000))
    }

    private fun identity(sessionId: String = "conversation-1") = AgentCoreRuntimeIdentity(
        hostId = "android-device:1234",
        clientId = "com.nola",
        sessionId = sessionId
    )

    private fun authorization(
        identity: AgentCoreRuntimeIdentity,
        nowMs: Long = System.currentTimeMillis()
    ) = AndroidAgentResultEgressAuthorization(
        route = "device",
        identity = identity,
        issuedAtMs = nowMs,
        expiresAtMs = nowMs + 60_000
    )

    private fun snapshot(isForeground: Boolean = true): AndroidAgentPermissionSnapshot =
        AndroidAgentPermissionSnapshot(
            capturedAtMs = System.currentTimeMillis(),
            authorizations = AndroidAgentToolRegistry.capabilities.associateWith { "ask" },
            availableCapabilities = AndroidAgentToolRegistry.capabilities,
            isForeground = isForeground,
            scopedGrants = emptyList()
        )

    private fun output(
        input: AgentCoreToolPolicyInput,
        candidate: AndroidAgentToolCandidate,
        capability: String
    ): AgentCoreToolPolicyOutput = AgentCoreToolPolicyOutput(
        schemaVersion = 1,
        platform = "android",
        policyDigest = "a".repeat(64),
        manifestDigest = "b".repeat(64),
        manifestValidUntilMs = input.runtime.expiresAtMs,
        enabledTools = listOf(
            AgentCoreResolvedTool(
                toolId = candidate.toolId,
                name = candidate.toolId,
                enabled = true,
                route = "device",
                capability = capability,
                permission = "scoped_resource",
                authorization = "ask",
                risk = "read_only",
                requiresResultEgress = true,
                reason = "enabled",
                schema = candidate.schema
            )
        )
    )
}
