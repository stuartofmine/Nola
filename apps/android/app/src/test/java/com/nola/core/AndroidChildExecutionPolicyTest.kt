package com.nola.core

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidChildExecutionPolicyTest {
    private fun config(rules: List<JsonObject>, egress: JsonElement) = buildJsonObject {
        put("permission_rules", buildJsonObject { put("rules", JsonArray(rules)) })
        put("tools", buildJsonArray { add(buildJsonObject {
            put("x_nola", buildJsonObject { put("requires_result_egress", egress) })
        }) })
    }

    @Test fun frozenPolicyDoesNotFollowChangedComposerMode() {
        for (original in AndroidAgentPermissionMode.entries) {
            val admitted = config(listOf(checkNotNull(original.corePermissionRule())), JsonPrimitive(true))
            for (currentComposerMode in AndroidAgentPermissionMode.entries) {
                // The current composer is deliberately not an input to restoring this Run.
                val observed = androidChildExecutionPolicy(admitted)
                assertEquals(original, observed.permissionMode)
                assertTrue(observed.resultEgressAuthorized)
                if (currentComposerMode != original) assertNotEquals(currentComposerMode, observed.permissionMode)
            }
        }
    }

    @Test fun unknownOrConflictingOriginalModeIsRejected() {
        assertThrows(IllegalArgumentException::class.java) { androidChildExecutionPolicy(config(emptyList(), JsonPrimitive(false))) }
        assertThrows(IllegalArgumentException::class.java) {
            androidChildExecutionPolicy(config(AndroidAgentPermissionMode.entries.map { checkNotNull(it.corePermissionRule()) }, JsonPrimitive(false)))
        }
    }

    @Test fun egressRequiresAnAdmittedBooleanFact() {
        val rules = listOf(checkNotNull(AndroidAgentPermissionMode.Cautious.corePermissionRule()))
        assertFalse(androidChildExecutionPolicy(config(rules, JsonPrimitive(false))).resultEgressAuthorized)
        assertThrows(IllegalArgumentException::class.java) { androidChildExecutionPolicy(config(rules, JsonPrimitive("true"))) }
    }
}
