package com.nola.data.local

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import kotlin.test.*

class RemoteTaskGuidanceIntentTest {
    private fun intent(id: String = "message", time: Long = 10) = buildJsonObject {
        put("hostId","host"); put("conversationId","parent"); put("parentRunId","run"); put("taskId","task")
        put("messageId",id); put("submittedAtMs",time); put("message","Instruction")
    }
    private fun receipt(value: JsonObject, status: String = "delivered") = buildJsonObject {
        for (key in listOf("hostId","conversationId","taskId","messageId")) put(key,value.getValue(key))
        put("guidance",buildJsonObject {
            put("task_id",value.getValue("taskId")); put("message_id",value.getValue("messageId"))
            put("owner_session_id",value.getValue("conversationId")); put("content",value.getValue("message"))
            put("submitted_at_ms",value.getValue("submittedAtMs")); put("status",status)
        })
    }
    @Test fun lostResponseRecoversOriginalIdAndTimeWithoutResubmission() = runTest {
        var saved: String? = null
        assertFailsWith<IllegalStateException> {
            submitRecoverableTaskGuidance(intent(),{saved},{saved=it}) { _,_ -> error("lost") }
        }
        var calls=0
        val status=submitRecoverableTaskGuidance(intent("new",99),{saved},{saved=it}) { frozen, query ->
            calls++; assertTrue(query); assertEquals(intent(),frozen);receipt(frozen)
        }
        assertEquals(1,calls); assertEquals("delivered",status);assertNull(saved)
    }
    @Test fun absentReceiptResendsFrozenIntentAndQueryFailureNeverResends() = runTest {
        var saved: String? = intent().toString();var calls=0
        assertFailsWith<IllegalStateException> {
            submitRecoverableTaskGuidance(intent(),{saved},{saved=it}) { _, query -> calls++;assertTrue(query);error("offline") }
        }
        assertEquals(1,calls);assertNotNull(saved)
        val status=submitRecoverableTaskGuidance(intent("other",999),{saved},{saved=it}) { frozen,query ->
            assertEquals(intent(),frozen)
            if(query) JsonObject(receipt(frozen)+("guidance" to JsonNull)) else receipt(frozen,"accepted")
        }
        assertEquals("accepted",status);assertNull(saved)
    }
    @Test fun foreignReceiptDoesNotClearOriginalIntent() = runTest {
        var saved: String? = intent().toString()
        assertFailsWith<IllegalArgumentException> {
            submitRecoverableTaskGuidance(intent(),{saved},{saved=it}) { frozen,_ ->
                JsonObject(receipt(frozen)+("hostId" to JsonPrimitive("foreign")))
            }
        }
        assertNotNull(saved)
    }
}
