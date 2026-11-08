package com.nola.data.local

import java.io.File
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class ExpertHostIntentJournalTest {
    @get:Rule val temporary = TemporaryFolder()
    private fun command() = buildJsonObject {
        put("type", "native_expert_host_operation")
        put("operation", buildJsonObject { put("kind", "prepare_edited_resend"); put("input", buildJsonObject { put("operation_id", "stable-business-id") }) })
    }
    private fun prepare(journal: ApplicationSubmissionJournal) = journal.prepare(command(), 1, null, null)
    private fun response(request: JsonObject, pending: Boolean) = buildJsonObject {
        put("ok", false)
        put("error", buildJsonObject {
            put("request_id", request.getValue("request_id")); put("code", if (pending) "expert_host_outcome_pending" else "not_authorized")
            put("message", "Owner reconciliation required"); put("outcome_pending", pending); put("reopen_required", pending)
        })
    }.toString().encodeToByteArray()

    @Test fun unknownAndPendingKeepTheExactOriginalEnvelopeAcrossRestart() {
        val file = File(temporary.root, "private-host.json")
        val journal = ApplicationSubmissionJournal(file)
        val original = prepare(journal)
        try { journal.acceptExpertHostResponse(original, null); fail("Missing ACK cannot retire an intent") }
        catch (_: ApplicationSubmissionJournal.PendingSubmission) { }
        journal.acceptExpertHostResponse(original, response(original, true))
        val reopened = ApplicationSubmissionJournal(file)
        assertEquals(listOf(original), reopened.pending())
        assertEquals(original, prepare(reopened))
    }

    @Test fun definitiveDenialAllowsNewTransportAttemptForTheSameBusinessIntent() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "private-host.json"))
        val first = prepare(journal)
        journal.acceptExpertHostResponse(first, response(first, false))
        assertTrue(journal.pending().isEmpty())
        val next = prepare(journal)
        assertNotEquals(first["request_id"], next["request_id"])
        assertEquals(first["command"], next["command"])
    }

    @Test fun foreignSuccessAndMalformedTerminalCannotAcknowledgeThisIntent() {
        val journal = ApplicationSubmissionJournal(File(temporary.root, "private-host.json"))
        val request = prepare(journal)
        val foreign = buildJsonObject { put("ok", true); put("request_id", "foreign"); put("result", JsonNull) }
        val malformed = buildJsonObject { put("ok", false); put("error", buildJsonObject { put("request_id", request.getValue("request_id")) }) }
        for (receipt in listOf(foreign, malformed)) {
            try { journal.acceptExpertHostResponse(request, receipt.toString().encodeToByteArray()); fail("Uncertain receipt cannot retire intent") }
            catch (_: ApplicationSubmissionJournal.PendingSubmission) { }
            assertEquals(listOf(request), journal.pending())
        }
    }

    @Test fun rendererJournalCannotBecomeAHostOperationOnRestart() {
        val ordinary = ApplicationSubmissionJournal(File(temporary.root, "ordinary.json"))
        val attempted = prepare(ordinary)
        val host = ApplicationSubmissionJournal(File(temporary.root, "expert-host.json"))
        assertTrue(host.pending().isEmpty())
        assertEquals(listOf(attempted), ApplicationSubmissionJournal(File(temporary.root, "ordinary.json")).pending())
        val actual = prepare(host)
        assertNotEquals(attempted["client_id"], actual["client_id"])
        assertNotEquals(attempted["request_id"], actual["request_id"])
    }
}
