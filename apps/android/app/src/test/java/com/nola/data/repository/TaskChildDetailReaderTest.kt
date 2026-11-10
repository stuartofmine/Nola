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
    @Test fun userConversationCannotBeOpenedAsTaskDetail() = runBlocking {
        val result = runCatching {
            readTaskChildDetail("parent","run","task","child",null,false) {
                Json.parseToJsonElement("""{"projection":{"execution_ownership":{"kind":"user"}}}""")
            }
        }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }
}
