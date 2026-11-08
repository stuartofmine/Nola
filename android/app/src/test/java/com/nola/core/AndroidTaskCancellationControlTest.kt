package com.nola.core

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidTaskCancellationControlTest {
    private val request = Json.parseToJsonElement("""{"effect_id":"effect","request_id":"request","payload":{"type":"cancel_task","task_id":"child"},"delivery":{"lease_token":"old"}}""").jsonObject
    private val delivery = Json.parseToJsonElement("""{"lease_token":"current"}""")
    @Test fun currentFenceGoesToCoreAndRequestedDoesNotBecomeCancelled() {
        val response = executeAndroidTaskCancellation(request, delivery, 42) { command ->
            assertEquals(JsonPrimitive("execute_task_cancellation_for_effect"), command["type"])
            assertEquals(delivery, command.getValue("request").jsonObject["delivery"])
            assertEquals(JsonPrimitive(42), command["now_ms"])
            Json.parseToJsonElement("""{"kind":"applied","cancel_requested":true}""")
        }
        assertEquals(JsonPrimitive("completed"), response["type"])
        assertEquals(JsonPrimitive(true), response.getValue("output").jsonObject["cancel_requested"])
        assertFalse(response.toString().contains("cancelled"))
    }
    @Test fun transferWithoutTrustedHostTransportFailsInsteadOfClaimingSuccess() {
        val response = executeAndroidTaskCancellation(request, delivery, 42) {
            Json.parseToJsonElement("""{"kind":"transfer","authorization":{}}""")
        }
        assertEquals(JsonPrimitive("failed"), response["type"])
        assertEquals(JsonPrimitive("task_cancellation_transfer_unsupported"), response.getValue("error").jsonObject["code"])
    }
    @Test fun staleFenceCoreRejectionCannotProduceSuccessfulResponse() {
        assertThrows(IllegalStateException::class.java) {
            executeAndroidTaskCancellation(request, delivery, 42) { error("stale delivery fence") }
        }
        assertThrows(IllegalArgumentException::class.java) {
            executeAndroidTaskCancellation(request, delivery, 42) {
                Json.parseToJsonElement("""{"kind":"applied","cancel_requested":"false"}""")
            }
        }
    }
}
