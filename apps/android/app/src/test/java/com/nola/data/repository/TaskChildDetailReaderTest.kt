package com.nola.data.repository
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class TaskChildDetailReaderTest {
    @Test fun foreignOwnerIsRejectedBeforeAnyJournalBodyRead() = runBlocking {
        var calls = 0
        val result = runCatching {
            readTaskChildDetail("parent","run","task","child",null,true) { request ->
                calls++
                assertEquals("task_child_window", request.getValue("operation").jsonPrimitive.content)
                assertEquals("run", request.getValue("parent_run_id").jsonPrimitive.content)
                Json.parseToJsonElement("""{"projection":{"execution_ownership":{"kind":"task_child","parent_conversation_id":"foreign","parent_run_id":"run","task_id":"task","child_conversation_id":"child"}}}""")
            }
        }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
        assertEquals(1,calls)
    }
    @Test fun expertTaskRejectsAnExchangeFromADifferentParentRun() = runBlocking {
        val result = runCatching {
            readTaskChildDetail("parent", "current-run", "exchange", "child", null, true,
                expertCallId = "call", expertQuery = { command ->
                    assertEquals("child_history", command["type"]?.jsonPrimitive?.content)
                    Json.parseToJsonElement("""{"version":1,"conversation_id":"parent","child_conversation_id":"child","room_id":"room","expert_exchanges":[{"exchange_id":"exchange","parent_conversation_id":"parent","source_run_id":"other-run","source_call_id":"call","room_id":"room","target":{"context_conversation_id":"child"}}]}""").jsonObject
                }) { error("No journal body should be read for a foreign exchange") }
        }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }

    @Test fun userConversationCannotBeOpenedAsTaskDetail() = runBlocking {
        val result = runCatching {
            readTaskChildDetail("parent","run","task","child",null,false) {
                Json.parseToJsonElement("""{"projection":{"execution_ownership":{"kind":"user"}}}""")
            }
        }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }
}
