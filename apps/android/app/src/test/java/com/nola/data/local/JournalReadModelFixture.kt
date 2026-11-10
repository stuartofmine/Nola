package com.nola.data.local

import kotlinx.serialization.json.*

/** Read-boundary fixture, not a producer or a replica persistence implementation. */
class JournalReadModelFixture(message: String = "assistant", run: String = "run") {
    val identity = buildJsonObject {
        put("source_store_epoch", "epoch"); put("conversation_id", "conversation")
        put("run_id", run); put("assistant_message_id", message)
    }
    var sequence = 1uL
    var digest = "a".repeat(64)
    val cursor get() = buildJsonObject {
        put("identity", identity); put("sequence", sequence.toString()); put("record_digest", digest)
    }
    val heads = linkedMapOf<String, JsonObject>()
    private val bodies = mutableMapOf<Pair<String, String>, String>()
    val operations = mutableListOf<String>()
    var foreignOwner: JsonObject? = null
    val reader = AndroidJournalPresentation(::execute)

    fun text(id: String, value: String, position: ULong = 1u, generation: ULong = 0u,
             visibility: String = "public", review: String? = null, final: Boolean = true) {
        head(id, "text", buildJsonObject {
            put("visibility", visibility); put("review", review); put("is_final", final)
        }, mapOf("markdown" to value), position, generation)
    }

    fun head(id: String, kind: String, metadata: JsonObject, content: Map<String, String> = emptyMap(),
             position: ULong = 1u, generation: ULong = 0u) {
        for ((field, text) in content) bodies[id to field] = text
        heads[id] = buildJsonObject {
            put("identity", identity); put("part_id", id); put("generation", generation.toString())
            put("revision", sequence.toString()); put("metadata_revision", sequence.toString())
            put("order_key", buildJsonObject { put("section", "transcript"); put("position", position.toString()) })
            put("metadata", buildJsonObject { put("kind", kind); put("data", metadata) })
            put("bodies", JsonArray(content.map { (field, text) -> buildJsonObject {
                put("field", field); put("content_generation", generation.toString())
                put("utf8_bytes", text.toByteArray().size.toString()); put("chunk_count", "1")
                put("content_root", java.security.MessageDigest.getInstance("SHA-256")
                    .digest(text.toByteArray()).joinToString("") { "%02x".format(it) })
            } }))
        }
    }

    fun execute(request: JsonObject): JsonElement {
        val operation = request.journalString("type"); operations += operation
        val published = heads.values.sortedBy { it.journalString("part_id") }.map { head ->
            foreignOwner?.let { JsonObject(head + ("identity" to it)) } ?: head
        }
        return when (operation) {
            "open_assistant_journal_snapshot" -> buildJsonObject { put("snapshot_id", "snapshot"); put("at", cursor) }
            "read_assistant_journal_parts" -> buildJsonObject {
                put("at", cursor); put("parts", JsonArray(published.sortedWith(compareBy<JsonObject> {
                    it.getValue("order_key").jsonObject.journalNumber("position")
                }.thenBy { it.journalString("part_id") }))); put("next_part_id", JsonNull)
            }
            "read_assistant_journal_part_changes" -> buildJsonObject {
                put("at", cursor); put("base", request.getValue("base")); put("next_part_id", JsonNull)
                put("changes", JsonArray(published.map { head -> buildJsonObject {
                    put("part_id", head.getValue("part_id")); put("head", head)
                } }))
            }
            "read_assistant_journal_range" -> {
                val bytes = bodies.getValue(request.journalString("part_id") to request.getValue("content").jsonObject.journalString("field")).toByteArray()
                val offset = request.journalNumber("offset_bytes").toInt()
                buildJsonObject {
                    put("offset_bytes", offset.toString()); put("next_offset_bytes", bytes.size.toString())
                    put("total_utf8_bytes", bytes.size.toString()); put("end_of_content", true)
                    put("text", bytes.copyOfRange(offset, bytes.size).decodeToString())
                }
            }
            "release_assistant_journal_snapshot" -> JsonNull
            else -> error("Unexpected read operation: $operation")
        }
    }
}
