package com.nola.data.local

import com.nola.domain.projection.ConversationProjectionV1
import kotlinx.serialization.json.*
import kotlin.test.*

class AndroidJournalReadModelContractTest {
    @Test fun reasoningAndToolMetadataAdvanceInPlaceInCanonicalOrder() {
        val source = JournalReadModelFixture()
        fun inline(value: String) = buildJsonObject { put("kind", "inline"); put("value", value) }
        source.head("reason", "reasoning", buildJsonObject {
            put("state", "running"); put("title", inline("Thinking"))
        }, mapOf("reasoning_summary" to "Checking"), position = 1u)
        fun tool(state: String, summary: String) = buildJsonObject {
            put("execution_source", buildJsonObject { put("kind", "model") })
            put("tool_call_id", "call"); put("capability_id", "web_search"); put("state", state)
            put("title", inline("Search")); put("summary", inline(summary)); put("display_key", JsonNull)
            put("origin", buildJsonObject { put("kind", "builtin") })
            put("permission", JsonNull); put("full_output", JsonNull)
            put("affected_resources", buildJsonObject { put("kind", "inline"); put("value", JsonArray(emptyList())) })
            put("artifacts", "tool_artifacts"); put("started_at_ms", "1"); put("finished_at_ms", if (state == "completed") "2" else null)
        }
        val bodies = mapOf("tool_input" to """{"protocol_version":1,"value":{"query":"Nola"}}""",
            "tool_artifacts" to """{"protocol_version":1,"artifacts":[]}""")
        source.head("tool", "tool", tool("running", "Searching"), bodies, position = 2u)
        val before = source.reader.partsFor(source.cursor, 1)
        assertEquals("Checking", (before.first() as ConversationProjectionV1.Part.Reasoning).summary)
        source.sequence++
        source.head("tool", "tool", tool("completed", "Found 2 sources"),
            bodies + ("tool_output_text" to "2 results"), position = 2u)
        val after = source.reader.partsFor(source.cursor, 2)
        assertEquals(listOf("reason", "tool"), after.map { it.partId })
        val completed = after.filterIsInstance<ConversationProjectionV1.Part.Tool>().single()
        assertEquals(ConversationProjectionV1.ActivityState.Completed, completed.state)
        assertEquals("Found 2 sources", completed.summary); assertEquals("2 results", completed.outputPreview)
        assertEquals("Checking", (after.first() as ConversationProjectionV1.Part.Reasoning).summary)
    }

    @Test fun stableIdentityAppendAndAuthoritativeReplacementUseCommittedHeads() {
        val source = JournalReadModelFixture()
        source.text("text", "Hello")
        val before = source.reader.partsFor(source.cursor, 1).single() as ConversationProjectionV1.Part.Text
        source.sequence++; source.text("text", "Hello world")
        val appended = source.reader.partsFor(source.cursor, 2).single() as ConversationProjectionV1.Part.Text
        assertEquals(before.partId, appended.partId); assertEquals("Hello", before.markdown)
        assertEquals("Hello world", appended.markdown)
        source.sequence++; source.text("text", "Replacement", generation = 1u)
        val replaced = source.reader.partsFor(source.cursor, 3)
        assertEquals(1, replaced.size); assertEquals("Replacement", (replaced.single() as ConversationProjectionV1.Part.Text).markdown)
    }

    @Test fun livePartsKeepStableOrderWithoutReplacingEarlierCommittedText() {
        val source = JournalReadModelFixture()
        source.text("z-first", "First ", position = 1u)
        source.text("a-second", "answer", position = 2u, final = false)
        val parts = source.reader.partsFor(source.cursor, 1).filterIsInstance<ConversationProjectionV1.Part.Text>()
        assertEquals(listOf("z-first", "a-second"), parts.map { it.partId })
        assertEquals("First answer", parts.joinToString("") { it.markdown })
        val successor = JournalReadModelFixture("next-assistant", "next-run")
        successor.text("next-text", "New answer")
        val next = successor.reader.partsFor(successor.cursor, 1).single() as ConversationProjectionV1.Part.Text
        assertEquals("New answer", next.markdown)
        assertEquals("First ", parts.first().markdown)
    }

    @Test fun staleCursorCannotRollBackAndSameClockDifferentDigestFailsClosed() {
        val source = JournalReadModelFixture()
        source.text("text", "Current")
        val old = source.cursor
        source.reader.partsFor(old, 1)
        source.sequence++; source.text("text", "Current answer")
        source.reader.partsFor(source.cursor, 2)
        assertEquals("Current answer", (source.reader.partsFor(old, 2).single() as ConversationProjectionV1.Part.Text).markdown)
        source.digest = "b".repeat(64)
        assertFails { source.reader.partsFor(source.cursor, 2) }
    }

    @Test fun foreignRunOrMessageHeadsAreRejectedAndSnapshotReleased() {
        for (field in listOf("source_store_epoch", "conversation_id", "run_id", "assistant_message_id")) {
            val source = JournalReadModelFixture()
            source.text("text", "Private")
            source.foreignOwner = JsonObject(source.identity + (field to JsonPrimitive("foreign")))
            assertFails { source.reader.partsFor(source.cursor, 1) }
            assertEquals("release_assistant_journal_snapshot", source.operations.last())
        }
    }

    @Test fun withheldReviewNeverReadsOrRendersPrivateBody() {
        for (review in listOf("pending", "rejected", "interrupted")) {
            val source = JournalReadModelFixture()
            source.text("draft", "Private draft", visibility = "withheld", review = review)
            val text = source.reader.partsFor(source.cursor, 1).single() as ConversationProjectionV1.Part.Text
            assertEquals("", text.markdown)
            assertFalse("read_assistant_journal_range" in source.operations)
        }
    }
}
