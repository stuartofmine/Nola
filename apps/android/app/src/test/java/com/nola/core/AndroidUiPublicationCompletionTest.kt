package com.nola.core

import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidUiPublicationCompletionTest {
    private val payload = buildJsonObject {
        put("call_id", "ui-call")
        put("capability_id", "render_ui")
        put("input", buildJsonObject { put("expected_revision", 1) })
    }

    @Test fun knownCasRejectionReachesModelWithExactRecoveryGuidanceAndOriginalInput() = runTest {
        val guidance = """{"error":"ui_revision_conflict","expected_revision":1,"current_revision":2,"guidance":"Review user edits; do not guess or overwrite."}"""
        val completion = androidUiPublicationCompletion(payload) { throw AndroidApplicationRuntime.RejectedMutation(guidance) }
        assertEquals("failed", completion.getValue("outcome").jsonPrimitive.content)
        val output = completion.getValue("output").jsonObject
        assertEquals("not_started", output.getValue("execution").jsonObject.getValue("state").jsonPrimitive.content)
        val data = output.getValue("data").jsonObject
        assertEquals(guidance, data.getValue("error").jsonObject.getValue("message").jsonPrimitive.content)
        assertEquals(payload.getValue("input"), data.getValue("requested_call").jsonObject.getValue("arguments"))
    }

    @Test fun committedPublicationReturnsActualReceipt() = runTest {
        val receipt = buildJsonObject { put("revision", 3); put("ready", true) }
        val completion = androidUiPublicationCompletion(payload) { receipt }
        assertEquals("succeeded", completion.getValue("outcome").jsonPrimitive.content)
        assertEquals(receipt, completion.getValue("output").jsonObject.getValue("data"))
    }

    @Test fun unknownAndCancellationNeverBecomeFailedOrSuccessfulToolResponses() = runTest {
        for (error in listOf(AndroidApplicationRuntime.PendingMutation("pending"), CancellationException("stop"))) {
            try {
                androidUiPublicationCompletion(payload) { throw error }
                fail("Unknown or cancelled publication must propagate")
            } catch (actual: Exception) { assertSame(error, actual) }
        }
    }
}
