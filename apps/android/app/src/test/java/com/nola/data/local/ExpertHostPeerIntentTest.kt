// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertHostPeerIntentTest {
    private fun operation(requestId: String = "transport-1", effect: String = "effect-1", exchange: String = "exchange-1") = buildJsonObject {
        put("kind", "prepare_peer_task"); put("exchange_id", exchange)
        put("request", buildJsonObject {
            put("request_id", requestId); put("effect_id", effect); put("session_id", "source"); put("turn_id", "run")
        })
    }
    @Test fun transportRecoveryDoesNotRecaptureAnAlreadyFrozenBusinessEffect() {
        assertEquals(expertHostIntentIdentity(operation()), expertHostIntentIdentity(operation(requestId = "recovery-2")))
        assertNotEquals(expertHostIntentIdentity(operation()), expertHostIntentIdentity(operation(effect = "different-effect")))
        assertNotEquals(expertHostIntentIdentity(operation()), expertHostIntentIdentity(operation(exchange = "different-target-run")))
    }
    @Test fun preparedReceiptMustMatchBothExchangeAndTargetContext() {
        fun receipt(task: String, child: String) = buildJsonObject {
            put("view", buildJsonObject { put("exchange_id", "exchange-1"); put("target", buildJsonObject { put("context_conversation_id", "target") }) })
            put("prepared", buildJsonObject { put("task", buildJsonObject { put("id", task); put("child_session_id", child) }) })
        }
        validateExpertPeerHostReceipt(operation(), receipt("exchange-1", "target"))
        for (bad in listOf(receipt("foreign", "target"), receipt("exchange-1", "source"))) {
            try { validateExpertPeerHostReceipt(operation(), bad); fail("Foreign worker receipt accepted") } catch (_: IllegalArgumentException) { }
        }
    }
}
