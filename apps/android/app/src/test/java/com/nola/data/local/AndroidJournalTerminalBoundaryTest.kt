package com.nola.data.local

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import kotlin.test.*

/** Terminal controls are hints; only a complete Application-verified seal commits a Run. */
class AndroidJournalTerminalBoundaryTest {
    private val identity = buildJsonObject {
        put("source_store_epoch", "epoch"); put("conversation_id", "conversation")
        put("run_id", "run"); put("assistant_message_id", "assistant")
    }
    private val cursor = buildJsonObject { put("identity", identity); put("sequence", "1"); put("record_digest", "a".repeat(64)) }
    private fun turn() = AndroidJournalTurn("host", "conversation", "submission", "assistant", "run")
    private fun response(value: JsonElement) = buildJsonObject { put("source_host_id", "host"); put("result", value) }

    @Test fun missingSealNeverCommitsSyntheticCompletion() = runTest {
        val commands = mutableListOf<String>()
        val receiver = AndroidAssistantJournal { command ->
            commands += command.journalString("type")
            error("No Application mutation is justified by a missing seal")
        }
        val result = receiver.receiveTerminal(turn(), { request ->
            assertEquals("terminal_seal", request.journalString("operation"))
            response(JsonNull)
        }, { true })
        assertNull(result); assertTrue(commands.isEmpty())
    }

    @Test fun mismatchedTerminalSealAndForeignRunFailBeforeAnyCommit() = runTest {
        for (foreign in listOf(false, true)) {
            val commands = mutableListOf<String>()
            val receiver = AndroidAssistantJournal { command ->
                commands += command.journalString("type")
                error("Invalid seal cannot be staged")
            }
            val owner = if (foreign) JsonObject(identity + ("run_id" to JsonPrimitive("foreign"))) else identity
            val seal = buildJsonObject {
                put("identity", owner); put("admission", cursor); put("entry_count", "1")
                put("seal_digest", if (foreign) "b".repeat(64) else "c".repeat(64))
            }
            assertFails { receiver.receiveTerminal(turn(), { response(seal) }, { true }, "b".repeat(64), "run") }
            assertFalse("commit_received_assistant_run_terminal" in commands)
        }
    }
}
