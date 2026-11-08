package com.nola.core

import org.junit.Assert.*
import org.junit.Test

class AndroidFrozenToolPlanTest {
    private fun plan(expires: Long) = AgentCoreAndroidToolPlan(
        tools = emptyList(), grantedCapabilities = listOf("none"),
        allowedTools = listOf(AgentCoreAllowedAndroidTool("render_ui", "android.core.ui.publish", "none", "device", "none", "granted", "low", false)),
        policyDigest = "policy", manifestDigest = "manifest", manifestId = "manifest-id",
        manifestVersion = 1, manifestValidUntilMs = expires,
        identity = AgentCoreRuntimeIdentity("host", "client", "conversation"),
    )

    @Test fun longModelOrApprovalWaitRenewsReceiptWithoutChangingFrozenAuthority() {
        val frozen = plan(60_000)
        val fresh = plan(180_000).copy(manifestVersion = 2, manifestDigest = "renewed")
        assertFalse(frozen.receiptIsValid(120_001))
        assertEquals(fresh, revalidateFrozenAndroidToolPlan(frozen, fresh, 120_001))
    }

    @Test fun dispatchUsesFreshExactAuthorityAfterLongWait() {
        val now = System.currentTimeMillis()
        val frozen = plan(now - 60_000)
        val fresh = plan(now + 180_000).copy(manifestVersion = 2, manifestDigest = "renewed")
        for (kind in listOf("request_permission", "execute_tool")) {
            val dispatch = androidToolPlanForHostRequest(kind, frozen, { fresh }, now)
            assertTrue(dispatch.receiptIsValid(now))
            assertTrue(dispatch.allowsPermission("render_ui", "none", "none"))
            assertEquals("renewed", dispatch.manifestDigest)
            assertTrue(runCatching { androidToolPlanForHostRequest(kind, frozen,
                { fresh.copy(allowedTools = emptyList()) }, now) }.isFailure)
            assertTrue(runCatching { androidToolPlanForHostRequest(kind, frozen, null, now) }.isFailure)
        }
        assertSame(frozen, androidToolPlanForHostRequest("model_stream", frozen,
            { error("Unexpected native tool refresh") }, now))
    }

    @Test fun changedToolPermissionResourceOrIdentityCannotBeAdoptedByWaitingRun() {
        val frozen = plan(60_000)
        val fresh = plan(180_000)
        val changes = listOf(fresh.copy(allowedTools = emptyList()),
            fresh.copy(identity = AgentCoreRuntimeIdentity("foreign", "client", "conversation")),
            fresh.copy(scopedGrants = listOf(AndroidAgentScopedGrantFact("files_read", "device", "/new", 180_000))),
            fresh.copy(allowedTools = fresh.allowedTools.map { it.copy(permission = "agent_approval") }),
            fresh.copy(manifestValidUntilMs = 120_000))
        changes.forEach { changed ->
            assertTrue(runCatching { revalidateFrozenAndroidToolPlan(frozen, changed, 120_001) }.isFailure)
        }
    }
    @Test fun uiPublisherUsesBoundManifestAndCorePermissionDecisionWithoutDataEgress() {
        val schema = kotlinx.serialization.json.Json.parseToJsonElement("""{"type":"function","function":{"name":"render_ui","parameters":{"type":"object"}}}""")
        fun resolved(enabled: Boolean): AgentCoreAndroidToolPlan = AndroidAgentToolPolicyPlanner.plan(
            executionMode = AndroidAgentExecutionMode.DeviceLocal,
            snapshot = AndroidAgentPermissionSnapshot(System.currentTimeMillis(), mapOf("none" to "granted"), setOf("none"), true, emptyList()),
            handlerCapabilities = setOf("none"), handlerIds = setOf("android.core.ui.publish"),
            identity = AgentCoreRuntimeIdentity("host", "client", "conversation"), resultEgressAuthorization = null,
            uiToolSchema = schema,
            resolver = AgentCoreToolPolicyResolver { input ->
                val advertised = input.runtime.tools.single { it.toolId == "render_ui" }
                assertTrue(advertised.available)
                assertEquals(schema, advertised.schema)
                assertTrue(input.resultEgressGrants.isEmpty())
                val tool = AgentCoreResolvedTool("render_ui", "render_ui", enabled, "device", "none", "none", "granted", "low", false, "", schema)
                AgentCoreToolPolicyOutput(2, "android", "policy", "manifest", input.runtime.expiresAtMs,
                    enabledTools = if (enabled) listOf(tool) else emptyList(), disabledTools = if (enabled) emptyList() else listOf(tool))
            })
        val allowed = resolved(true)
        assertTrue(allowed.allowsPermission("render_ui", "none", "none"))
        assertFalse(allowed.allowedTools.single().requiresResultEgress)
        assertTrue(resolved(false).allowedTools.isEmpty())
        assertTrue(resolved(false).tools.isEmpty())
    }

}
