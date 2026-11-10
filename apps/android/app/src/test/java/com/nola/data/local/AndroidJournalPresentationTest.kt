package com.nola.data.local

import com.nola.domain.projection.ConversationProjectionV1
import kotlinx.serialization.json.*
import kotlin.test.*

class AndroidJournalPresentationTest {
    @Test fun uiSurfaceJournalPreservesPlainSummaryAndIndependentRevision() {
        val source = JournalReadModelFixture()
        source.sequence = 90uL
        source.head("ui-surface:hash", "ui_surface", Json.parseToJsonElement("""{
            "surface":{"identity":{"owner":{"authority_id":"host","conversation_id":"conversation",
            "participant_id":"agent","context_id":"context","originating_run_id":"run",
            "message_id":"assistant","part_id":"render-tool"},"surface_id":"form","generation":1},"revision":2},
            "summary":"填写姓名"
        }""").jsonObject)
        val part = source.reader.partsFor(source.cursor, 90).single() as ConversationProjectionV1.Part.UiSurface
        assertEquals("填写姓名", part.summary)
        assertEquals(2L, part.surface.revision)
        assertEquals("render-tool", part.surface.identity.owner.part_id)
    }

    @Test fun coldDirectoryUsesDisplayOrderAndOpaqueRetiredContinuationIncludingEmptyPages() {
        val source = JournalReadModelFixture()
        source.text("z-text", "First", position = 1u)
        source.head("a-reason", "reasoning", buildJsonObject {
            put("state", "completed"); put("title", buildJsonObject { put("kind", "inline"); put("value", "Thinking") })
        }, mapOf("reasoning_summary" to "Second"), position = 2u)
        val visited = mutableListOf<String?>()
        val reader = AndroidJournalPresentation { request ->
            if (request.journalString("type") != "read_assistant_journal_parts") source.execute(request)
            else {
                val after = request["after_part_id"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
                visited += after
                val (parts, next) = when (after) {
                    null -> listOf(source.heads.getValue("z-text")) to "z-retired"
                    "z-retired" -> emptyList<JsonObject>() to "b-retired"
                    "b-retired" -> listOf(source.heads.getValue("a-reason")) to "a-retired"
                    "a-retired" -> emptyList<JsonObject>() to null
                    else -> error("Unexpected continuation")
                }
                // Application wraps Store's page with its pinned snapshot at.
                buildJsonObject { put("at", source.cursor); put("parts", JsonArray(parts)); put("next_part_id", next) }
            }
        }
        val parts = reader.partsFor(source.cursor, 1)
        assertEquals("First", (parts[0] as ConversationProjectionV1.Part.Text).markdown)
        assertEquals("Second", (parts[1] as ConversationProjectionV1.Part.Reasoning).summary)
        assertEquals(listOf(null, "z-retired", "b-retired", "a-retired"), visited)
    }

    @Test fun coldDirectoryRejectsCursorCyclesForeignSnapshotAndRegressingDisplayOrder() {
        for (failure in listOf("cycle", "snapshot", "order")) {
            val source = JournalReadModelFixture()
            source.text("a", "later", position = 2u)
            source.text("z", "earlier", position = 1u)
            val reader = AndroidJournalPresentation { request ->
                if (request.journalString("type") != "read_assistant_journal_parts") source.execute(request)
                else buildJsonObject {
                    put("at", if (failure == "snapshot") JsonObject(source.cursor + ("sequence" to JsonPrimitive("9"))) else source.cursor)
                    put("parts", if (failure == "order") JsonArray(source.heads.values.toList()) else JsonArray(emptyList()))
                    put("next_part_id", if (failure == "cycle") JsonPrimitive("retired") else JsonNull)
                }
            }
            assertFails(failure) { reader.partsFor(source.cursor, 1) }
            assertEquals("release_assistant_journal_snapshot", source.operations.last())
        }
    }

    @Test fun incrementalChangesStillRequireLexicalPartOrderNotDisplayOrder() {
        val source = JournalReadModelFixture()
        source.text("z", "first", position = 1u)
        source.text("a", "second", position = 2u)
        val reader = AndroidJournalPresentation { request ->
            if (request.journalString("type") != "read_assistant_journal_part_changes") source.execute(request)
            else buildJsonObject {
                put("at", source.cursor); put("base", request.getValue("base")); put("next_part_id", JsonNull)
                put("changes", JsonArray(source.heads.values.map { head -> buildJsonObject {
                    put("part_id", head.getValue("part_id")); put("head", head)
                } }))
            }
        }
        assertEquals(2, reader.partsFor(source.cursor, 1).size)
        source.sequence++
        assertFails { reader.partsFor(source.cursor, 1) }
    }
    @Test fun displayReadsOnlyChangedHeadsAndNewUtf8BytesWithoutNarrowingJournalClocks() {
        val identity = buildJsonObject { put("source_store_epoch", "epoch"); put("conversation_id", "conversation"); put("run_id", "run"); put("assistant_message_id", "assistant") }
        fun cursor(sequence: String) = buildJsonObject { put("identity", identity); put("sequence", sequence); put("record_digest", "a".repeat(64)) }
        var at = cursor("18446744073709551614"); var text = "你"
        val operations = mutableListOf<String>(); val offsets = mutableListOf<String>()
        val reader = AndroidJournalPresentation { request ->
            val operation = request.journalString("type"); operations += operation
            val reference = buildJsonObject { put("field", "markdown"); put("content_generation", "0"); put("utf8_bytes", text.toByteArray().size.toString()); put("chunk_count", "1"); put("content_root", if (text == "你") "b".repeat(64) else "c".repeat(64)) }
            val head = buildJsonObject {
                put("identity", identity); put("part_id", "part"); put("generation", "0"); put("revision", at.getValue("sequence")); put("metadata_revision", "0")
                put("order_key", buildJsonObject { put("section", "transcript"); put("position", "18446744073709551615") })
                put("metadata", buildJsonObject { put("kind", "text"); put("data", buildJsonObject { put("visibility", "public"); put("is_final", false); put("review", JsonNull) }) })
                put("bodies", JsonArray(listOf(reference)))
            }
            when (operation) {
                "open_assistant_journal_snapshot" -> buildJsonObject { put("snapshot_id", "snapshot"); put("at", at) }
                "read_assistant_journal_parts" -> buildJsonObject { put("at", at); put("parts", JsonArray(listOf(head))); put("next_part_id", JsonNull) }
                "read_assistant_journal_part_changes" -> buildJsonObject { put("at", at); put("base", request.getValue("base")); put("changes", JsonArray(listOf(buildJsonObject { put("part_id", "part"); put("head", head) }))); put("next_part_id", JsonNull) }
                "read_assistant_journal_range" -> {
                    val offset = request.journalString("offset_bytes"); offsets += offset
                    buildJsonObject { put("offset_bytes", offset); put("next_offset_bytes", text.toByteArray().size.toString()); put("total_utf8_bytes", text.toByteArray().size.toString()); put("text", text.toByteArray().copyOfRange(offset.toInt(), text.toByteArray().size).decodeToString()); put("end_of_content", true) }
                }
                "release_assistant_journal_snapshot" -> JsonNull
                else -> error(operation)
            }
        }
        assertEquals("你", (reader.partsFor(at, 23).single() as ConversationProjectionV1.Part.Text).markdown)
        at = cursor("18446744073709551615"); text = "你好🙂"
        val part = reader.partsFor(at, 23).single() as ConversationProjectionV1.Part.Text
        assertEquals("你好🙂", part.markdown); assertEquals(23L, part.revision, "UI revision belongs to the local read model, not the source journal clock")
        assertEquals(listOf("0", "3"), offsets)
        assertEquals(1, operations.count { it == "read_assistant_journal_parts" })
        assertEquals(1, operations.count { it == "read_assistant_journal_part_changes" })
    }
}
