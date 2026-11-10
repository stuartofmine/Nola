package com.nola.data.local

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import kotlin.test.*

class AndroidAssistantJournalTest {
    @Test fun sealedDisplayedRootUsesDurableSourceReceiptInsteadOfPendingOrPermissionProjection() {
        val identity = Json.parseToJsonElement("""{"source_host_id":"host","client_id":"client","conversation_id":"conversation","submission_id":"run","user_message_id":"user","assistant_message_id":"assistant"}""").jsonObject
        val admission = Json.parseToJsonElement("""{"identity":{"source_store_epoch":"epoch","conversation_id":"conversation","run_id":"run","assistant_message_id":"assistant"},"sequence":"1","record_digest":"${"a".repeat(64)}"}""").jsonObject
        var foreign = false
        val commands = mutableListOf<String>()
        val journal = AndroidAssistantJournal { command ->
            commands += command.journalString("type")
            when (command.journalString("type")) {
                "load_conversation_projection_window_v1" -> buildJsonObject {
                    assertEquals(JsonPrimitive(1),command["limit"])
                    put("assistant_journal_heads",JsonArray(listOf(admission)))
                }
                "read_admitted_remote_run_identity" -> {
                    assertEquals("run",command.journalString("run_id"))
                    if (foreign) JsonObject(identity + ("client_id" to JsonPrimitive("other"))) else identity
                }
                else -> error("must not infer a sealed source from transient projections")
            }
        }
        assertEquals("run",journal.admittedSourceOwner("conversation","host","client","assistant")?.submissionId)
        assertEquals(listOf("load_conversation_projection_window_v1","read_admitted_remote_run_identity"),commands)
        foreign = true
        assertFails { journal.admittedSourceOwner("conversation","host","client","assistant") }
        assertNull(journal.admittedSourceOwner("conversation","host","client",null))
    }

    @Test fun contextHintFetchesExactAdmittedRecordAndPassesOpaqueCountersToCore() = runTest {
        val admission = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        val usage = buildJsonObject {
            put("protocol_version", 1); put("identity", identity); put("admission", admission)
            put("attempt_sequence", "1"); put("run_cache_usage", JsonNull)
            put("event_sequence", "9007199254740993"); put("record_digest", "c".repeat(64))
            put("usage", buildJsonObject {
                put("measurement", JsonNull); put("prompt_cache_measurement", JsonNull); put("prompt_cache_diagnostic", JsonNull)
                put("used_tokens", "7500"); put("context_window_tokens", "32768")
                put("prompt_token_limit", "28672"); put("estimated", false)
            })
        }
        val calls = mutableListOf<String>()
        val journal = AndroidAssistantJournal { command ->
            calls += command.journalString("type")
            when (command.journalString("type")) {
                "run_message_admission_reception_state" -> admission
                "accept_remote_run_context_usage" -> {
                    assertEquals("host", command.journalString("source_host_id"))
                    assertEquals(usage, command["record"], "Pass the source record unchanged to Core validation")
                    usage
                }
                else -> error("Unexpected command $command")
            }
        }
        val remote: suspend (JsonObject) -> JsonObject = { query ->
            calls += query.journalString("operation")
            response(when (query.journalString("operation")) {
                "admission_head" -> admission
                "context_usage" -> { assertEquals(identity, query["identity"]); usage }
                else -> error("Unexpected query $query")
            })
        }
        journal.receiveContextUsage(owner().copy(runId = "canonical-run"), remote, { true })
        assertTrue(calls.indexOf("run_message_admission_reception_state") < calls.indexOf("accept_remote_run_context_usage"))
        assertEquals(1, calls.count { it == "accept_remote_run_context_usage" })
    }

    @Test fun contextResponseCannotCommitAfterObserverGenerationChangesOrFromAnotherHost() = runTest {
        val admission = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        for (changeGeneration in listOf(false, true)) {
            var current = true
            var commits = 0
            val journal = AndroidAssistantJournal { query ->
                if (query.journalString("type") == "accept_remote_run_context_usage") commits++
                admission
            }
            val remote: suspend (JsonObject) -> JsonObject = { query ->
                if (query.journalString("operation") == "admission_head") response(admission)
                else {
                    if (changeGeneration) current = false
                    buildJsonObject { put("source_host_id", if (changeGeneration) "host" else "foreign-host"); put("result", JsonNull) }
                }
            }
            assertFails { journal.receiveContextUsage(owner().copy(runId = "canonical-run"), remote, { current }) }
            assertEquals(0, commits)
        }
    }

    @Test fun observedContextCannotReplaceTheAdmittedRoot() = runTest {
        val admission = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        var commits = 0
        val journal = AndroidAssistantJournal { command ->
            if (command.journalString("type") == "accept_remote_run_context_usage") commits++
            admission
        }
        val foreign = buildJsonObject {
            put("identity", identity plusJson mapOf("assistant_message_id" to JsonPrimitive("other-root")))
            put("admission", admission)
        }
        assertFails {
            journal.receiveContextUsage(owner().copy(runId = "canonical-run"),
                { error("Observed record already includes its admission") }, { true }, foreign)
        }
        assertEquals(0, commits)
    }
    private val identity = buildJsonObject { put("source_store_epoch", "epoch"); put("conversation_id", "conversation"); put("run_id", "canonical-run"); put("assistant_message_id", "submission:assistant") }
    private val cursor = buildJsonObject { put("identity", identity); put("sequence", "18446744073709551615"); put("record_digest", "a".repeat(64)) }
    private fun owner() = AndroidJournalTurn("host", "conversation", "submission", "submission:assistant")
    private fun response(result: JsonElement) = buildJsonObject { put("source_host_id", "host"); put("result", result) }
    private fun terminalSeal(admission: JsonObject) = buildJsonObject {
        put("protocol_version", 1); put("identity", identity); put("admission", admission)
        put("core_session_id", "core"); put("transition_id", "terminal-transition")
        put("checkpoint_revision", "2"); put("core_revision", "2"); put("event_cursor", "9")
        put("completed_at_ms", 2); put("entry_count", "1"); put("heads_count", "1"); put("child_receipt_count", "0")
        put("manifest_digest", "c".repeat(64)); put("seal_digest", "b".repeat(64)); put("result_digest", "d".repeat(64))
        put("result", buildJsonObject {
            put("result_id", "decision"); put("conversation_id", "conversation"); put("run_id", "canonical-run")
            put("originating_submission_id", "submission")
            put("input", buildJsonObject { put("watermark", "1"); put("applications_digest", "e".repeat(64)) })
            put("decided_at_revision", "2"); put("decision_event_cursor", "8")
            put("requirement_revision", "1"); put("evidence_frontier", "7")
            put("value", buildJsonObject { put("kind", "answered"); put("answer_part_refs", JsonArray(listOf(JsonPrimitive("answer")))) })
        })
    }
    private val record = buildJsonObject { put("header", cursor plusJson mapOf("protocol_version" to JsonPrimitive(1))); put("batch", buildJsonObject {}) }

    @Test fun countersRetainUnsignedPrecisionAndRejectNumericJson() {
        assertEquals(ULong.MAX_VALUE, cursor.journalNumber("sequence"))
        assertFails { buildJsonObject { put("sequence", 1) }.journalNumber("sequence") }
        assertFails { buildJsonObject { put("sequence", "01") }.journalNumber("sequence") }
    }

    @Test fun activeColdRunReceivesAdmissionBeforeHeadsWithoutAnyLocalHeadOrTerminalSeal() = runTest {
        val calls = mutableListOf<String>()
        val admission = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        val initial = admission plusJson mapOf("sequence" to JsonPrimitive("0"), "record_digest" to JsonPrimitive("0".repeat(64)))
        val journal = AndroidAssistantJournal { command ->
            val type = command.journalString("type"); calls += type
            when (type) {
                "run_message_admission_reception_state" -> initial
                "receive_run_message_admissions" -> admission
                "assistant_journal_reception_state" -> buildJsonObject { put("after", initial); put("complete", true) }
                else -> error(type)
            }
        }
        val remote: suspend (JsonObject) -> JsonObject = { request ->
            val operation = request.journalString("operation"); calls += operation
            response(when (operation) {
                "admissions" -> buildJsonObject {
                    put("after", initial); put("through", admission); put("next", admission); put("complete", true)
                    put("records", JsonArray(listOf(buildJsonObject { put("cursor", admission) })))
                }
                "heads" -> JsonArray(emptyList())
                else -> error("Cold active catch-up must not require a seal: $operation")
            })
        }
        val turn = owner().copy(runId = "canonical-run")
        val received = journal.catchUpRun(turn, remote, { true }, admission)
        assertTrue(received.first.isEmpty()); assertFalse(received.second)
        assertTrue(calls.indexOf("receive_run_message_admissions") < calls.indexOf("heads"))
        assertFalse("terminal_seal" in calls)
    }

    @Test fun terminalMessagesRequireTheirOwnStateAndCompletionTime() {
        val message = buildJsonObject { put("state", "completed"); put("created_at_ms", 1); put("completed_at_ms", 2) }
        message.requireTerminalMessage()
        assertFails { JsonObject(message - "state").requireTerminalMessage() }
        assertFails { JsonObject(message - "completed_at_ms").requireTerminalMessage() }
        assertFails { (message plusJson mapOf("state" to JsonPrimitive("running"))).requireTerminalMessage() }
    }

    @Test fun unknownOwnerReceivesBoundedAdmissionsBeforeJournalAndCachesThem() = runTest {
        val calls = mutableListOf<String>()
        val admission = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        val initial = admission plusJson mapOf("sequence" to JsonPrimitive("0"), "record_digest" to JsonPrimitive("0".repeat(64)))
        var admitted = false
        val journal = AndroidAssistantJournal { request ->
            val type = request.journalString("type"); calls += type
            when (type) {
                "run_message_admission_reception_state" -> if (admitted) admission else initial
                "receive_run_message_admissions" -> { admitted = true; admission }
                "assistant_journal_reception_state" -> {
                    assertTrue(admitted)
                    buildJsonObject { put("after", cursor); put("complete", true) }
                }
                else -> error(type)
            }
        }
        val remote: suspend (JsonObject) -> JsonObject = { request ->
            val op = request.journalString("operation"); calls += op
            response(when (op) {
                "admission_head" -> admission
                "admissions" -> buildJsonObject {
                    put("after", initial); put("through", admission); put("next", admission); put("complete", true)
                    put("records", JsonArray(listOf(buildJsonObject { put("cursor", admission) })))
                }
                else -> error(op)
            })
        }
        val owner = owner()
        journal.receive(cursor, owner, remote, { true })
        journal.receive(cursor, owner, remote, { true })
        assertEquals(1, calls.count { it == "admission_head" })
        assertTrue(calls.indexOf("receive_run_message_admissions") < calls.indexOf("assistant_journal_reception_state"))
    }

    @Test fun recordContentIsStagedBeforeDurableReceptionAndSnapshotIsReleased() = runTest {
        val calls = mutableListOf<String>(); var staged = false; var received = false
        val need = buildJsonObject {
            put("part_id", "part"); put("content", buildJsonObject { put("utf8_bytes", "3"); put("chunk_count", "1") })
            put("expected_prefix", buildJsonObject { put("utf8_bytes", "0"); put("chunk_count", "0") })
        }
        val initial = buildJsonObject { put("identity", identity); put("sequence", "0"); put("record_digest", "0".repeat(64)) }
        val journal = AndroidAssistantJournal { command ->
            val type = command.journalString("type"); calls += type
            when (type) {
                "run_message_admission_reception_state" -> command.getValue("through")
                "assistant_journal_reception_state" -> buildJsonObject { put("complete", received); put("after", if (received) cursor else initial) }
                "next_assistant_journal_content" -> if (staged) JsonNull else need
                "stage_assistant_journal_content" -> { staged = true; need.getValue("content") }
                "receive_assistant_journal_record" -> { assertTrue(staged); received = true; buildJsonObject { put("cursor", cursor) } }
                else -> error(type)
            }
        }
        journal.receive(cursor, owner(), { request ->
            val op = request.journalString("operation"); calls += op
            response(when (op) {
                "admission_head" -> cursor
                "read" -> buildJsonObject { put("status", "records"); put("after", initial); put("through", cursor); put("next", cursor); put("complete", true); put("records", JsonArray(listOf(record))) }
                "open_snapshot" -> buildJsonObject { put("snapshot_id", "snapshot"); put("at", cursor) }
                "record_chunk" -> buildJsonObject { put("text", "你") }
                "release_snapshot" -> JsonNull
                else -> error(op)
            })
        }, { true })
        assertTrue(received)
        assertTrue(calls.indexOf("stage_assistant_journal_content") < calls.indexOf("receive_assistant_journal_record"))
        assertTrue(calls.indexOf("release_snapshot") > calls.indexOf("receive_assistant_journal_record"))
    }

    @Test fun terminalStagesImmutableManifestBeforeJournalsAndCommitsOnlyAfterCatchup() = runTest {
        val calls = mutableListOf<String>()
        val seal = terminalSeal(cursor)
        var staged = false
        val journal = AndroidAssistantJournal { request ->
            val type = request.journalString("type"); calls += type
            when (type) {
                "run_message_admission_reception_state" -> request.getValue("through")
                "begin_receive_assistant_run_terminal", "stage_assistant_run_terminal_manifest", "commit_received_assistant_run_terminal" -> {
                    if (type == "stage_assistant_run_terminal_manifest") staged = true
                    if (type == "commit_received_assistant_run_terminal") assertTrue("assistant_journal_reception_state" in calls)
                    buildJsonObject { put("seal", seal); put("next_entry", if (staged) "1" else "0"); put("manifest_complete", staged); put("committed", type == "commit_received_assistant_run_terminal") }
                }
                "assistant_journal_reception_state" -> { assertTrue(staged); buildJsonObject { put("after", cursor); put("complete", true) } }
                else -> error(type)
            }
        }
        val result = journal.receiveTerminal(owner(), { request -> response(when (request.journalString("operation")) {
            "admission_head" -> cursor
            "terminal_seal" -> seal
            "context_usage" -> JsonNull
            "terminal_manifest" -> buildJsonObject { put("seal", seal); put("after_entry", "0"); put("next_entry", "1"); put("complete", true); put("entries", JsonArray(listOf(buildJsonObject { put("kind", "assistant"); put("through", cursor); put("state", "completed"); put("created_at_ms", 1); put("completed_at_ms", 2) }))) }
            else -> error("Unexpected query")
        }) }, { true }, "b".repeat(64), "canonical-run")
        assertEquals(listOf(cursor), result)
        assertEquals("commit_received_assistant_run_terminal", calls.last())
    }

    @Test fun foreignSourceOrStaleUiCannotWriteReplica() = runTest {
        var writes = 0
        val journal = AndroidAssistantJournal { writes++; error("Must not write") }
        assertFailsWith<IllegalStateException> { journal.receive(cursor, owner(), { error("Must not request") }, { false }) }
        assertEquals(0, writes)
        assertFailsWith<IllegalArgumentException> {
            journal.receiveTerminal(owner(), { buildJsonObject { put("source_host_id", "foreign"); put("result", JsonNull) } }, { true })
        }
        assertEquals(0, writes)
    }

    @Test fun visibleHistoryRecoversCurrentHeadsAfterImmutableTerminalFrontier() = runTest {
        val calls = mutableListOf<String>()
        val sealed = cursor plusJson mapOf("sequence" to JsonPrimitive("1"))
        val seal = terminalSeal(sealed)
        val journal = AndroidAssistantJournal { request ->
            val type = request.journalString("type"); calls += type
            if (type == "run_message_admission_reception_state") request.getValue("through")
            else if (type == "assistant_journal_reception_state") {
                calls += "read:${request.getValue("through").jsonObject.journalString("sequence") }"
                buildJsonObject { put("after", request.getValue("through")); put("complete", true) }
            } else {
                buildJsonObject { put("seal", seal); put("next_entry", "1"); put("manifest_complete", true); put("committed", true) }
            }
        }
        journal.recoverVisible("host", "conversation", listOf(sealed), { request ->
            val operation = request.journalString("operation"); calls += operation
            response(when (operation) {
                "admission_head" -> sealed
                "terminal_seal" -> seal
                "context_usage" -> JsonNull
                "terminal_manifest" -> buildJsonObject {
                    put("seal", seal); put("after_entry", "0"); put("next_entry", "1"); put("complete", true)
                    put("entries", JsonArray(listOf(buildJsonObject {
                        put("kind", "assistant"); put("through", sealed); put("state", "completed"); put("created_at_ms", 1); put("completed_at_ms", 2)
                    })))
                }
                "heads" -> JsonArray(listOf(cursor))
                else -> error(operation)
            })
        }, { true })
        assertTrue(calls.indexOf("commit_received_assistant_run_terminal") < calls.indexOf("heads"))
        assertTrue("read:18446744073709551615" in calls)
        assertEquals(1, calls.count { it == "heads" }, "must only enumerate the visible Run")
    }
}

private infix fun JsonObject.plusJson(values: Map<String, JsonElement>) = JsonObject(this + values)
