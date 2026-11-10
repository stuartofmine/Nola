package com.nola.data.remote.api

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class RemoteUiQueueTest {
    private fun wire(phase: String = "rejected", client: String = "client") = buildJsonObject {
        put("head", buildJsonObject { put("command_id", "action"); put("kind", "user_input"); put("user_message_id", "ui:action:user") })
        put("dispatch", buildJsonObject {
            put("protocol_version",1)
            put("identity",RemoteSubmissionIdentity("host",client,"conversation","action","ui:action:user","action:assistant").wire())
            put("request_digest","a".repeat(64)); put("phase",phase); put("admission",JsonNull); put("rejection_code","dispatch_not_admitted")
        })
    }
    @Test fun terminalReceiptRetainsExactIdentityAndDigest() {
        val value=decodeRemoteUiQueue(wire(),"host","client","conversation")
        assertEquals(RemoteSubmissionPhase.Rejected,value.dispatch?.phase)
        assertEquals("action",value.dispatch?.identity?.submissionId)
        assertEquals("a".repeat(64),value.dispatch?.requestDigest)
        assertEquals(RemoteSubmissionPhase.Prepared,decodeRemoteUiQueue(wire("prepared"),"host","client","conversation").dispatch?.phase)
    }
    @Test fun crossClientAndMissingHeadAreRejected() {
        assertThrows(IllegalArgumentException::class.java) { decodeRemoteUiQueue(wire(client="foreign"),"host","client","conversation") }
        assertThrows(IllegalStateException::class.java) { decodeRemoteUiQueue(JsonObject(wire()+ ("head" to JsonNull)),"host","client","conversation") }
    }
    @Test fun cancellationSettlesOriginalOwnerAfterNavigationWithoutClearingAnotherQueue() {
        val receipt=checkNotNull(decodeRemoteUiQueue(wire(),"host","client","conversation").dispatch)
        val rejected=mutableMapOf("conversation" to receipt)
        val suppressed=mutableSetOf("conversation","visible-other")
        clearCancelledUiQueueOwner(receipt,rejected,suppressed)
        assertFalse(rejected.containsKey("conversation"))
        assertFalse(suppressed.contains("conversation"))
        assertTrue(suppressed.contains("visible-other"))
        val newer=receipt.copy(requestDigest="b".repeat(64))
        rejected["conversation"]=newer; suppressed.add("conversation")
        clearCancelledUiQueueOwner(receipt,rejected,suppressed)
        assertEquals(newer,rejected["conversation"])
        assertTrue(suppressed.contains("conversation"))
    }
    @Test fun oldShapeIsNotSilentlyTreatedAsEmpty() {
        assertThrows(IllegalArgumentException::class.java) { decodeRemoteUiQueue(buildJsonObject { put("command_id","action") },"host","client","conversation") }
        assertNull(decodeRemoteUiQueue(buildJsonObject { put("head",JsonNull);put("dispatch",JsonNull) },"host","client","conversation").head)
    }
}
