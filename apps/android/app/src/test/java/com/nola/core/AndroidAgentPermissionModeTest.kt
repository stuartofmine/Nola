package com.nola.core

import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class AndroidAgentPermissionModeTest {
    @Test
    fun twoCanonicalValuesRejectAliasesAndDefaultToCautious() {
        assertEquals(
            listOf("cautious", "allow_all"),
            AndroidAgentPermissionMode.entries.map(AndroidAgentPermissionMode::wireValue),
        )
        assertEquals(AndroidAgentPermissionMode.Cautious, AndroidAgentPermissionMode.fromWireValue(null))
        for (value in listOf("safe", "recommended", "unknown", "请搜索当前设置", "moderate", "allowAll", "allow-all")) {
            assertThrows(IllegalArgumentException::class.java) { AndroidAgentPermissionMode.fromWireValue(value) }
        }
    }

    @Test
    fun modesMapToCoreAuthorityWithoutInspectingUserText() {
        val recommended = requireNotNull(AndroidAgentPermissionMode.Cautious.corePermissionRule())
        val allowAll = requireNotNull(AndroidAgentPermissionMode.AllowAll.corePermissionRule())

        assertEquals("allow", recommended["decision"]?.jsonPrimitive?.content)
        assertEquals("product_default", recommended["source"]?.jsonPrimitive?.content)
        assertEquals("allow", allowAll["decision"]?.jsonPrimitive?.content)
        assertEquals("invocation_policy", allowAll["source"]?.jsonPrimitive?.content)
    }

    @Test
    fun hostPlatformFactsRemainAuthoritativeAfterTheProductProfile() {
        val plan = AgentCoreAndroidToolPlan(
            tools = emptyList(),
            grantedCapabilities = emptyList(),
            allowedTools = listOf(
                AgentCoreAllowedAndroidTool(
                    toolId = "device_get_location",
                    handlerId = "android.location.current",
                    capability = "location",
                    route = "device",
                    permission = "system_permission",
                    authorization = "ask",
                    risk = "read_only",
                    requiresResultEgress = true,
                ),
            ),
            permissionMode = AndroidAgentPermissionMode.AllowAll,
        )

        val rules = androidAgentPermissionRules(plan)["rules"]!!.jsonArray
        assertEquals("invocation_policy", rules[0].jsonObject["source"]?.jsonPrimitive?.content)
        assertEquals("ask", rules[1].jsonObject["decision"]?.jsonPrimitive?.content)
        assertEquals("host", rules[1].jsonObject["source"]?.jsonPrimitive?.content)
    }
}
