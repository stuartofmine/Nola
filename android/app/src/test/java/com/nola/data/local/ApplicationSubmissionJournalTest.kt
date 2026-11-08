package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

class ApplicationSubmissionJournalTest {
    @Test fun readOnlyRejectionIsVisibleWithoutTreatingAWritingFailureAsTerminal() {
        val request = buildJsonObject { put("request_id", "query-rejected") }
        val failure = """{"ok":false,"error":{"code":"application_rejected","message":"model_execution_capabilities_required","recoverable":false}}""".encodeToByteArray()
        try { ApplicationReceiptValidation.decode(request, failure, readOnly = true); fail("Query rejection must surface") }
        catch (error: AndroidApplicationRuntime.RejectedMutation) { assertEquals("model_execution_capabilities_required", error.message) }
        try { ApplicationReceiptValidation.decode(request, failure); fail("A write still has no validated receipt") }
        catch (error: ApplicationSubmissionJournal.PendingSubmission) { assertEquals("query-rejected", error.requestId) }
    }

    @get:Rule val temporary = TemporaryFolder()
    private fun contextCommand(id: String, kind: String = "reduce_and_commit_active_submission") = buildJsonObject {
        put("type", kind)
        put("input", buildJsonObject { put("conversation_id", id) })
    }

    @Test fun failedModelObservationDoesNotKeepHistoryBlockedAfterRecovery() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "observation.json"))
        val query = journal.prepare(buildJsonObject {
            put("type", "resolve_remote_model_profile")
            put("profile_id", "missing"); put("expected_revision", 1)
            put("request", buildJsonObject {})
        }, 53, "query", null)
        journal.retireObservation(query)
        val write = journal.prepare(contextCommand("a"), 53, "write", null)
        try { journal.retireObservation(write); fail("A mutation must retain its receipt") }
        catch (_: IllegalArgumentException) { }
        assertEquals(listOf(write), journal.pending())
    }

    @Test fun historySnapshotCanOpenWhileSameConversationCancellationIsPending() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "snapshot-lanes.json"))
        val pending = journal.prepare(contextCommand("a"), 53, "cancel-a", null)
        val snapshot = journal.prepare(buildJsonObject {
            put("type", "open_assistant_journal_snapshot")
            put("at", buildJsonObject { put("identity", buildJsonObject { put("conversation_id", "a") }) })
        }, 53, "snapshot-a", null)
        journal.acceptResponse(snapshot, response(snapshot))
        assertEquals(listOf(pending), journal.pending())
        try {
            journal.prepare(contextCommand("a", "begin_assistant_turn"), 53, "next-a", null)
            fail("Snapshot reads must not unlock a new Run")
        } catch (failure: ApplicationSubmissionJournal.PendingSubmission) {
            assertEquals("cancel-a", failure.requestId)
        }
    }

    @Test fun independentContextSurvivesAnotherPendingCommandAndRestarts() {
        val file = File(temporary.root, "lanes.json")
        val first = ApplicationSubmissionJournal(file)
        val a = first.prepare(contextCommand("a"), 52, "request-a", null)
        val b = first.prepare(contextCommand("b"), 52, "request-b", null)
        val restarted = ApplicationSubmissionJournal(file)
        assertEquals(setOf(a, b), restarted.pending().toSet())
        try { restarted.prepare(contextCommand("a", "begin_assistant_turn"), 52, "next-a", null); fail("Same owner must wait") }
        catch (pending: ApplicationSubmissionJournal.PendingSubmission) { assertEquals("request-a", pending.requestId) }
        restarted.acceptResponse(b, response(b))
        assertEquals(listOf(a), restarted.pending())
        val nextB = restarted.prepare(contextCommand("b", "begin_assistant_turn"), 52, "next-b", null)
        restarted.acceptResponse(a, response(a, "rejected"))
        assertEquals(listOf(nextB), restarted.pending())
    }

    @Test fun unknownAndGlobalCommandsCannotBypassContextPending() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "global.json"))
        journal.prepare(contextCommand("a"), 52, "a", null)
        for (command in listOf(command(), contextCommand("b", "unknown_command"))) {
            try { journal.prepare(command, 52, null, null); fail("Global ownership must be conservative") }
            catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        }
    }

    @Test fun malformedRecoveredReceiptRetainsItsOwnerAndDoesNotPreventOtherRecovery() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "recover-lanes.json"))
        val a = journal.prepare(contextCommand("a"), 52, "a", null)
        val b = journal.prepare(contextCommand("b"), 52, "b", null)
        val recovered = mutableListOf<String>()
        for (request in journal.pending()) {
            try {
                val receipt = if (request == a) buildJsonObject {} else
                    Json.parseToJsonElement(response(request).decodeToString()).jsonObject.getValue("result").jsonObject
                journal.acceptReceipt(request, ApplicationReceiptValidation.recovered(request, receipt))
                recovered += request.getValue("request_id").jsonPrimitive.content
            } catch (_: AndroidApplicationRuntime.PendingMutation) { }
        }
        assertEquals(listOf("b"), recovered)
        assertEquals(listOf(a), journal.pending())
    }

    @Test fun runOnlyCommandsResolveToCanonicalConversationOwner() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "runs.json")) { run ->
            mapOf("run-a" to "a", "run-b" to "b")[run]
        }
        journal.prepare(contextCommand("a"), 52, "a", null)
        fun claim(run: String) = buildJsonObject {
            put("type", "claim_committed_host_requests")
            put("input", buildJsonObject { put("run_id", run) })
        }
        try { journal.prepare(claim("run-a"), 52, "claim-a", null); fail("Run belongs to blocked context") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        val b = journal.prepare(claim("run-b"), 52, "claim-b", null)
        journal.acceptResponse(b, response(b))
        assertEquals(1, journal.pending().size)
        try { journal.prepare(claim("missing"), 52, "missing", null); fail("Unknown Run cannot bypass") }
        catch (_: IllegalArgumentException) {}
    }
    private fun command(value: String = "原始命令") = buildJsonObject {
        put("type", "put_product_setting")
        put("key", "test")
        put("value_json", value)
    }
    private fun response(request: JsonObject, status: String = "committed") = buildJsonObject {
        put("ok", true)
        put("result", buildJsonObject {
            for (key in listOf("client_id", "request_id", "protocol_version")) put(key, request.getValue(key))
            put("status", status)
            put("received_at", "2026-10-12T00:00:00Z")
            if (status != "accepted") put("completed_at", "2026-10-12T00:00:01Z")
            if (status == "committed") put("result", JsonNull)
            if (status == "conflict") put("conflict", buildJsonObject {
                put("scope_id", "scope"); put("expected_revision", 2); put("actual_revision", 3)
            })
            if (status == "rejected") put("error", buildJsonObject {
                put("code", "rejected"); put("message", "rejected"); put("recoverable", false)
            })
        })
    }.toString().encodeToByteArray()

    @Test fun unknownOutcomeSurvivesRestartWithExactEnvelopeAndStableClient() {
        val file = File(temporary.root, "pending.json")
        val first = ApplicationSubmissionJournal(file)
        val request = first.prepare(command(), 12, null, 2)
        for (unknown in listOf(null, "broken".encodeToByteArray(), response(request, "accepted"))) {
            try { first.acceptResponse(request, unknown); fail("Expected pending") }
            catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        }
        val restarted = ApplicationSubmissionJournal(file)
        assertEquals(first.clientId, restarted.clientId)
        try { restarted.prepare(command(), 13, null, 2); fail("Changed protocol must not reuse an old envelope") }
        catch (_: IllegalArgumentException) {}
        assertEquals(request, restarted.prepare(command(), 12, null, 2))
        assertEquals(listOf(request), restarted.pending())
        restarted.acceptResponse(request, response(request))
        assertTrue(ApplicationSubmissionJournal(file).pending().isEmpty())
    }

    @Test fun changedPendingRequestAndMismatchedReceiptNeverReplaceOriginal() {
        val file = File(temporary.root, "pending.json")
        val journal = ApplicationSubmissionJournal(file)
        val request = journal.prepare(command(), 12, "request-1", null)
        try { journal.prepare(command("changed"), 12, "request-1", null); fail("Expected refusal") }
        catch (_: IllegalArgumentException) {}
        val unrelated = JsonObject(request + ("request_id" to JsonPrimitive("another")))
        try { journal.acceptResponse(request, response(unrelated)); fail("Expected pending") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        assertEquals(listOf(request), ApplicationSubmissionJournal(file).pending())
    }

    @Test fun unknownFirstWriteBlocksLaterWriteAcrossRestartUntilTerminalRecovery() {
        val file = File(temporary.root, "pending.json")
        val first = ApplicationSubmissionJournal(file)
        val create = first.prepare(command("create"), 12, "z-create", null)
        val restarted = ApplicationSubmissionJournal(file)
        try {
            restarted.prepare(command("edit"), 12, "a-edit", null)
            fail("Later write must not overtake unknown create")
        } catch (pending: ApplicationSubmissionJournal.PendingSubmission) {
            assertEquals("z-create", pending.requestId)
        }
        assertEquals(listOf(create), ApplicationSubmissionJournal(file).pending())
        assertEquals(create, restarted.prepare(command("create"), 12, null, null))
        restarted.acceptResponse(create, response(create))
        val edit = restarted.prepare(command("edit"), 12, "a-edit", null)
        assertEquals(listOf(edit), ApplicationSubmissionJournal(file).pending())
    }

    @Test fun onlyTerminalOutcomesRemovePendingRequests() {
        val file = File(temporary.root, "pending.json")
        val journal = ApplicationSubmissionJournal(file)
        for (status in listOf("committed", "conflict", "rejected")) {
            val request = journal.prepare(command(), 12, status, null)
            assertEquals(status, journal.acceptResponse(request, response(request, status))["status"]?.jsonPrimitive?.content)
            assertTrue(ApplicationSubmissionJournal(file).pending().isEmpty())
        }
        val request = journal.prepare(command(), 12, null, null)
        val unknown = """{"ok":false,"error":{"message":"unknown","recoverable":true}}""".encodeToByteArray()
        try { journal.acceptResponse(request, unknown); fail("Expected pending") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        assertEquals(1, ApplicationSubmissionJournal(file).pending().size)
        val rejected = """{"ok":false,"error":{"message":"invalid","recoverable":false}}""".encodeToByteArray()
        try { journal.acceptResponse(request, rejected); fail("Outer error is not a rejection receipt") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        assertEquals(listOf(request), ApplicationSubmissionJournal(file).pending())
    }

    @Test fun malformedReceiptAndInvalidBusinessReplyRetainExactIntent() {
        val file = File(temporary.root, "pending.json")
        val journal = ApplicationSubmissionJournal(file)
        val request = journal.prepare(command(), 48, "fixed-request", null)
        val good = Json.parseToJsonElement(response(request).decodeToString()).jsonObject
        val receipt = good.getValue("result").jsonObject
        val invalid = JsonObject(good + ("result" to JsonObject(receipt - "completed_at")))
        try { journal.acceptResponse(request, invalid.toString().encodeToByteArray()); fail("Missing terminal time") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        assertEquals(listOf(request), ApplicationSubmissionJournal(file).pending())
        try {
            journal.acceptReceipt(request, receipt) { error("Recovery reply belongs to a different Run") }
            fail("Business reply must validate before retirement")
        } catch (_: IllegalStateException) {}
        assertEquals(listOf(request), ApplicationSubmissionJournal(file).pending())
    }

    @Test fun staleInstanceReloadsPendingInsteadOfOverwritingIt() {
        val file = File(temporary.root, "pending.json")
        val first = ApplicationSubmissionJournal(file)
        val stale = ApplicationSubmissionJournal(file)
        assertTrue(stale.pending().isEmpty())
        val request = first.prepare(command(), 48, "original", null)
        try { stale.prepare(command("replacement"), 48, "new", null); fail("Must retain original") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) {}
        assertEquals(listOf(request), first.pending())
        assertEquals(first.clientId, stale.clientId)
    }

    @Test fun transactionCoversObservationAndRetirementAcrossInstances() {
        val file = File(temporary.root, "pending.json")
        val first = ApplicationSubmissionJournal(file)
        val second = ApplicationSubmissionJournal(file)
        val attempted = CountDownLatch(1)
        val acquired = CountDownLatch(1)
        val finished = CountDownLatch(1)
        val error = AtomicReference<Throwable?>()
        first.transaction {
            val request = first.prepare(command(), 48, "original", null)
            Thread {
                attempted.countDown()
                try {
                    second.transaction {
                        acquired.countDown()
                        assertTrue(second.pending().isEmpty())
                    }
                } catch (failure: Throwable) { error.set(failure) }
                finally { finished.countDown() }
            }.start()
            assertTrue(attempted.await(1, TimeUnit.SECONDS))
            assertFalse(acquired.await(50, TimeUnit.MILLISECONDS))
            first.acceptResponse(request, response(request))
        }
        assertTrue(finished.await(2, TimeUnit.SECONDS))
        error.get()?.let { throw AssertionError("Second journal transaction failed", it) }
    }

    @Test fun malformedJournalsNeverBecomeEmptyState() {
        for ((index, bytes) in listOf("null", "", "{broken").withIndex()) {
            val file = File(temporary.root, "bad-$index.json")
            file.writeText(bytes)
            java.nio.file.Files.setPosixFilePermissions(file.toPath(),
                java.nio.file.attribute.PosixFilePermissions.fromString("rw-------"))
            try { ApplicationSubmissionJournal(file).pending(); fail("Bad state must not create a replacement client") }
            catch (_: Exception) {}
            assertEquals(bytes, file.readText())
        }
    }
}
