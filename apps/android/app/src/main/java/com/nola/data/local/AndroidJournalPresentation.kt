package com.nola.data.local

import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.ConversationProjectionV1Decoder
import kotlinx.serialization.json.*

/** Local verified journal read model. Journal clocks remain u64 decimal strings;
 * public card revisions use the receiving conversation's independent local clock. */
class AndroidJournalPresentation(private val execute: (JsonObject) -> JsonElement) {
    private data class Body(val reference: JsonObject, val text: String)
    private data class Part(val head: JsonObject, val bodies: Map<String, Body>)
    private data class Snapshot(val at: JsonObject, val parts: Map<String, Part>)
    private val states = mutableMapOf<JsonObject, Snapshot>()

    @Synchronized
    fun overlay(projection: ConversationProjectionV1, cursors: List<JsonObject>): ConversationProjectionV1 {
        val grouped = cursors.groupBy { it.getValue("identity").jsonObject.journalString("assistant_message_id") }
        return projection.copy(messages = projection.messages.map { message ->
            val heads = grouped[message.messageId] ?: return@map message
            require(message.role == ConversationProjectionV1.Role.Assistant)
            message.copy(parts = heads.flatMap { cursor ->
                require(cursor.getValue("identity").jsonObject.journalString("conversation_id") == projection.conversationId)
                parts(cursor).map { present(it, message.messageId, message.revision) }
            })
        })
    }

    @Synchronized
    fun partsFor(through: JsonObject, localRevision: Long): List<ConversationProjectionV1.Part> =
        parts(through).map { present(it, through.getValue("identity").jsonObject.journalString("assistant_message_id"), localRevision) }

    private fun parts(at: JsonObject): List<Part> {
        val owner = at.getValue("identity").jsonObject
        val previous = states[owner]
        if (previous != null && at.journalNumber("sequence") <= previous.at.journalNumber("sequence")) {
            require(at.journalNumber("sequence") != previous.at.journalNumber("sequence") || at == previous.at)
            return ordered(previous.parts.values)
        }
        val snapshot = execute(journalCommand("open_assistant_journal_snapshot") { put("at", at) }).jsonObject
        try {
            require(snapshot["at"] == at)
            val parts = previous?.parts?.toMutableMap() ?: mutableMapOf()
            val seen = mutableSetOf<String>(); var after: String? = null
            val continuations = mutableSetOf<String>()
            var lastColdHead: JsonObject? = null
            do {
                val page = execute(journalCommand(if (previous == null) "read_assistant_journal_parts" else "read_assistant_journal_part_changes") {
                    put("snapshot", snapshot); put("after_part_id", after); put("max_records", 64); put("max_encoded_bytes", 262144)
                    previous?.let { put("base", it.at) }
                }).jsonObject
                require(page["at"] == at && (previous == null || page["base"] == previous.at))
                val entries = page.getValue(if (previous == null) "parts" else "changes").jsonArray
                require(entries.size <= 64)
                var lastChanged = after
                for (entry in entries) {
                    val value = entry.jsonObject
                    val head = if (previous == null) value else value.getValue("head").takeUnless { it == JsonNull }?.jsonObject
                    val partId = value.journalString("part_id")
                    require(seen.add(partId))
                    if (previous != null) {
                        require(lastChanged == null || compareIds(checkNotNull(lastChanged), partId) < 0)
                        lastChanged = partId
                    }
                    if (head == null) { parts.remove(partId); continue }
                    require(head["identity"] == owner && head.journalString("part_id") == partId)
                    orderSection(head)
                    head.getValue("order_key").jsonObject.journalNumber("position")
                    if (previous == null) {
                        require(lastColdHead == null || compareHeads(checkNotNull(lastColdHead), head) < 0)
                        lastColdHead = head
                    }
                    for (field in listOf("generation", "revision", "metadata_revision")) head.journalNumber(field)
                    val old = parts[partId]
                    val metadata = head.getValue("metadata").jsonObject
                    val withheld = metadata.journalString("kind") == "text" && metadata.getValue("data").jsonObject.journalString("visibility") != "public"
                    val bodies = mutableMapOf<String, Body>()
                    if (!withheld) for (item in head.getValue("bodies").jsonArray) {
                        val reference = item.jsonObject; val field = reference.journalString("field")
                        require(field !in bodies)
                        val cached = old?.takeIf { it.head["generation"] == head["generation"] }?.bodies?.get(field)
                            ?.takeIf { it.reference["content_generation"] == reference["content_generation"] }
                        var offset = cached?.reference?.journalNumber("utf8_bytes") ?: 0uL
                        val size = reference.journalNumber("utf8_bytes"); require(offset <= size)
                        if (cached != null && offset == size) require(cached.reference["content_root"] == reference["content_root"])
                        val text = StringBuilder(cached?.text.orEmpty())
                        while (offset < size) {
                            val range = execute(journalCommand("read_assistant_journal_range") {
                                put("snapshot", snapshot); put("part_id", partId); put("content", reference)
                                put("offset_bytes", offset.toString()); put("limit_bytes", 32768)
                            }).jsonObject
                            require(range.journalNumber("offset_bytes") == offset && range.journalNumber("total_utf8_bytes") == size)
                            val chunk = range.getValue("text").jsonPrimitive.content
                            val count = chunk.toByteArray(Charsets.UTF_8).size
                            val next = range.journalNumber("next_offset_bytes")
                            require(count in 1..32768 && next > offset && next - offset == count.toULong() && next <= size)
                            require(range.journalBoolean("end_of_content") == (next == size))
                            text.append(chunk); offset = next
                        }
                        bodies[field] = Body(reference, text.toString())
                    }
                    parts[partId] = Part(head, bodies)
                }
                val next = page.getValue("next_part_id").takeUnless { it == JsonNull }?.let {
                    require(it.jsonPrimitive.isString && it.jsonPrimitive.content.isNotBlank())
                    it.jsonPrimitive.content
                }
                if (next != null) {
                    require(continuations.add(next)) { "Journal directory continuation cycle" }
                    if (previous != null) require(entries.isNotEmpty() && next == lastChanged &&
                        (after == null || compareIds(checkNotNull(after), next) < 0))
                    // Cold continuation is the last scanned directory key,
                    // including retired entries; it need not name a returned head.
                }
                after = next
            } while (after != null)
            states[owner] = Snapshot(at, parts)
            return ordered(parts.values)
        } finally { execute(journalCommand("release_assistant_journal_snapshot") { put("snapshot", snapshot) }) }
    }

    private fun orderSection(head: JsonObject): Int = listOf("transcript", "tasks", "skills", "model_attempts")
        .indexOf(head.getValue("order_key").jsonObject.journalString("section")).also { require(it >= 0) }

    private fun compareIds(left: String, right: String): Int {
        val lhs = left.toByteArray(Charsets.UTF_8); val rhs = right.toByteArray(Charsets.UTF_8)
        for (index in 0 until minOf(lhs.size, rhs.size)) {
            val comparison = (lhs[index].toInt() and 255).compareTo(rhs[index].toInt() and 255)
            if (comparison != 0) return comparison
        }
        return lhs.size.compareTo(rhs.size)
    }

    private fun compareHeads(left: JsonObject, right: JsonObject): Int {
        val section = orderSection(left).compareTo(orderSection(right))
        if (section != 0) return section
        val position = left.getValue("order_key").jsonObject.journalNumber("position")
            .compareTo(right.getValue("order_key").jsonObject.journalNumber("position"))
        return if (position != 0) position else compareIds(left.journalString("part_id"), right.journalString("part_id"))
    }

    private fun ordered(parts: Collection<Part>): List<Part> = parts.sortedWith { left, right -> compareHeads(left.head, right.head) }

    private fun present(part: Part, messageId: String, localRevision: Long): ConversationProjectionV1.Part {
        val metadata = part.head.getValue("metadata").jsonObject
        val originalKind = metadata.journalString("kind")
        val data = metadata.getValue("data").jsonObject
        val fields = (if (originalKind == "task_control") data.getValue("task").jsonObject else data).toMutableMap()
        if (originalKind == "task_control") {
            fields["supports_guidance"] = data.getValue("supports_guidance")
            fields["supports_cancellation"] = data.getValue("supports_cancellation")
            fields["parent_run_id"] = part.head.getValue("identity").jsonObject.getValue("run_id")
        }
        var kind = originalKind
        fun body(field: String): String = part.bodies[field]?.text.orEmpty()
        fun text(value: JsonElement?): JsonElement = if (value == null || value == JsonNull) JsonNull else value.jsonObject.let {
            JsonPrimitive(if (it.journalString("kind") == "inline") it.getValue("value").jsonPrimitive.content else body(it.journalString("value")))
        }
        fun structured(field: String, key: String): JsonElement = part.bodies[field]?.text?.let {
            val value = Json.parseToJsonElement(it).jsonObject
            require(value.getValue("protocol_version").jsonPrimitive.int == 1 && value.keys == setOf("protocol_version", key))
            value.getValue(key)
        } ?: JsonNull
        fun number(value: JsonElement?): JsonElement = value?.takeUnless { it == JsonNull }?.let {
            val decimal = it.jsonPrimitive.content; require(decimal.toULong().toString() == decimal)
            Json.parseToJsonElement(decimal) // Exact JSON lexeme, never Double or signed narrowing.
        } ?: JsonNull
        for (key in listOf("title", "summary", "uri", "mime_type", "name", "operation", "display_name")) {
            if (kind == "ui_surface" && key == "summary") continue
            fields[key]?.let { fields[key] = text(it) }
        }
        when (kind) {
            "ui_surface" -> fields["summary"] = data.getValue("summary")
            "text" -> { fields.remove("visibility"); fields["markdown"] = JsonPrimitive(if (data.journalString("visibility") == "public"
                && (data["review"] == null || data["review"] == JsonNull || data["review"]?.jsonPrimitive?.content == "approved")) body("markdown") else "") }
            "reasoning" -> fields["summary"] = JsonPrimitive(body("reasoning_summary"))
            "tool" -> {
                fields["input"] = structured("tool_input", "value"); fields["output"] = structured("tool_output", "value")
                fields["output_preview"] = JsonPrimitive(body("tool_output_text")); fields["artifacts"] = structured("tool_artifacts", "artifacts")
                fields["affected_resources"] = data.getValue("affected_resources").jsonObject.let {
                    if (it.journalString("kind") == "inline") it.getValue("value") else structured("tool_resources", "resources")
                }
                for ((container, keys) in listOf("origin" to listOf("provider_id", "provider_title", "namespace"), "permission" to listOf("permission_domain", "rationale", "decision"))) {
                    data[container]?.takeUnless { it == JsonNull }?.jsonObject?.let { nested ->
                        fields[container] = JsonObject(nested.toMutableMap().apply { for (key in keys) if (key in this) this[key] = text(this[key]) })
                    }
                }
                fields["started_at_ms"] = number(data["started_at_ms"]); fields["finished_at_ms"] = number(data["finished_at_ms"])
            }
            "source" -> { fields["snippet"] = JsonPrimitive(body("source_snippet")); fields["evidence"] = structured("source_evidence", "evidence") }
            "citation" -> {
                val location = data["location"]?.takeUnless { it == JsonNull }?.jsonObject
                fields["location"] = location?.let { JsonObject(it.toMutableMap().apply {
                    for (key in listOf("page", "line_start", "line_end")) this[key] = number(this[key])
                    this["section"] = text(this["section"])
                }) } ?: JsonNull
                val converted = fields["location"]?.takeUnless { it == JsonNull }?.jsonObject
                for (key in listOf("page", "line_start", "line_end", "section")) fields[key] = converted?.get(key) ?: JsonNull
                val range = fields.remove("byte_range")?.takeUnless { it == JsonNull }?.jsonObject
                fields["range_start"] = number(range?.get("start")); fields["range_end"] = number(range?.get("end"))
            }
            "task", "task_control" -> {
                fields["current_step"] = JsonPrimitive(body("task_current_step")); fields["result_ref"] = JsonPrimitive(body("result_reference"))
                fields["progress"] = fields["progress"]?.takeUnless { it == JsonNull }?.jsonObject?.let {
                    val progress = Double.fromBits(it.journalNumber("fraction_bits").toLong()); require(progress.isFinite() && progress in 0.0..1.0); JsonPrimitive(progress)
                } ?: JsonNull
            }
            "skill" -> {
                for (key in listOf("model_turns", "tool_calls", "elapsed_millis")) fields[key] = number(fields[key])
                fields["result_ref"] = JsonPrimitive(body("result_reference")); fields["failure"] = JsonPrimitive(body("failure_message"))
            }
            "model_attempt" -> fields["failure"] = buildJsonObject {
                put("code", data.getValue("failure_code")); put("message", body("failure_message")); put("retryable", data.getValue("retryable"))
            }
            "historical" -> {
                // Historical bodies are read-only preview, never reinterpreted as executable tools.
                kind = if (data.journalString("canonical_kind") == "reasoning") "reasoning" else "text"
                fields.clear()
                if (kind == "text") { fields["markdown"] = JsonPrimitive(body("historical_preview")); fields["is_final"] = JsonPrimitive(true) }
                else { fields["summary"] = JsonPrimitive(body("historical_preview")); fields["title"] = JsonPrimitive(""); fields["state"] = JsonPrimitive("completed") }
            }
            "step" -> { kind = "reasoning"; fields["summary"] = JsonPrimitive("") }
        }
        fields["kind"] = JsonPrimitive(kind); fields["part_id"] = part.head.getValue("part_id"); fields["revision"] = JsonPrimitive(localRevision)
        return ConversationProjectionV1Decoder.decodeDisplayPart(JsonObject(fields), messageId)
    }
}
