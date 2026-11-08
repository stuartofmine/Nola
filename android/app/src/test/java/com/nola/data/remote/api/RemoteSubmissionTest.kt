package com.nola.data.remote.api

import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.decodeSsePayload
import kotlinx.serialization.json.*
import kotlin.test.*

class RemoteSubmissionTest {
    private val identity = RemoteSubmissionIdentity("host", "client", "conversation", "submission", "user", "assistant")
    private val digest = "a".repeat(64)
    private val request = buildJsonObject {
        put("conversationId", "conversation"); put("expected_host_id", "host"); put("client_id", "client")
        put("submission_id", "submission"); put("user_message_id", "user"); put("assistant_message_id", "assistant")
        put("messages", JsonArray(listOf(buildJsonObject { put("role", "user"); put("content", "原文🙂") })))
        put("language", "zh-Hans"); put("timezone", "Asia/Shanghai")
    }
    private val admission = buildJsonObject {
        put("identity", buildJsonObject { put("source_store_epoch", "epoch"); put("conversation_id", "conversation")
            put("run_id", "submission"); put("assistant_message_id", "assistant") })
        put("sequence", "2"); put("record_digest", "b".repeat(64))
    }
    private fun receipt(phase: String) = buildJsonObject {
        put("protocol_version", 1); put("identity", identity.wire()); put("request_digest", digest); put("phase", phase)
        put("admission", if (phase == "admitted") admission else JsonNull); put("rejection_code", JsonNull)
    }

    @Test fun frozenOriginalRequestAddsOnlyTheBoundRustDigest() {
        val frozen = FrozenRemoteSubmission(identity, digest, request)
        assertEquals(request, JsonObject(frozen.wireRequest() - "request_digest"))
        assertEquals("原文🙂", frozen.request.getValue("messages").jsonArray[0].jsonObject.getValue("content").jsonPrimitive.content)
        assertEquals(buildJsonObject { put("operation", "observe"); put("identity", identity.wire()); put("request_digest", digest) }, frozen.reconciliationRequest())
        assertEquals("cancel", frozen.reconciliationRequest(true).getValue("operation").jsonPrimitive.content)
        assertFails { FrozenRemoteSubmission(identity.copy(clientId = "other"), digest, request) }
        assertFails { FrozenRemoteSubmission(identity, digest, frozen.wireRequest()) }
    }
    @Test fun preparedReceiptDoesNotAdmitAndOnlyExactSourceReceiptCanCreateRoot() {
        val prepared = decodeRemoteSubmissionReceipt(receipt("prepared"), identity, digest)
        assertNull(prepared.admission); assertEquals(RemoteSubmissionPhase.Prepared, prepared.phase)
        val admitted = decodeRemoteSubmissionReceipt(receipt("admitted"), identity, digest)
        assertEquals(admission, admitted.admission)
        for (changed in listOf(identity.copy(sourceHostId = "other"), identity.copy(clientId = "other"),
            identity.copy(userMessageId = "other"), identity.copy(assistantMessageId = "other")))
            assertFails { decodeRemoteSubmissionReceipt(receipt("admitted"), changed, digest) }
        assertFails { decodeRemoteSubmissionReceipt(receipt("admitted"), identity, "c".repeat(64)) }
        assertFails { decodeRemoteSubmissionReceipt(JsonObject(receipt("admitted") + ("admission" to JsonNull)), identity, digest) }
        assertFails { decodeRemoteSubmissionReceipt(JsonObject(receipt("admitted") + ("admission" to
            JsonObject(admission + ("sequence" to JsonPrimitive("0"))))), identity, digest) }
    }
    @Test fun sourceReceiptSseUsesOnlyTheNestedNewContract() {
        val event = decodeSsePayload(buildJsonObject { put("type", "submission_receipt"); put("receipt", receipt("admitted")) }.toString(), null) as SseEvent.SubmissionReceipt
        assertEquals(receipt("admitted"), event.receipt)
        assertFails { decodeSsePayload(JsonObject(receipt("admitted") + ("type" to JsonPrimitive("submission_receipt"))).toString(), null) }
    }
    @Test fun coldAdoptionRefusesAnotherOriginClientOrUnadmittedSource() {
        val key = RemoteRunKey("conversation", "submission", "assistant")
        assertEquals(RemoteSubmissionPhase.Admitted, decodeObservedSubmissionReceipt(receipt("admitted"), "host", "client", key).phase)
        assertFails { decodeObservedSubmissionReceipt(receipt("admitted"), "host", "other", key) }
        assertFails { decodeObservedSubmissionReceipt(receipt("prepared"), "host", "client", key) }
        val unbound = decodeRemoteSubmissionIntent(buildJsonObject { put("identity", identity.wire()); put("request_digest", JsonNull); put("phase", "prepared") })
        assertNull(unbound.requestDigest)
        assertFails { decodeRemoteSubmissionIntent(buildJsonObject {
            put("identity", identity.wire()); put("request_digest", JsonNull); put("phase", "admitted")
        }) }
    }
}
