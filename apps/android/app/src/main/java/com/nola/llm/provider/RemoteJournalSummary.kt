package com.nola.llm.provider

import com.nola.data.local.journalBoolean
import com.nola.data.local.journalNumber
import com.nola.data.local.journalString
import com.nola.data.local.requireTerminalMessage
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

/** Read-only source client for non-persisted LlmClient summaries. Completion is
 * proven by a fixed authenticated seal, never by EOF or a trace event. */
internal class RemoteJournalSummary(private val host: String, private val conversation: String,
    private val submission: String, private val remote: suspend (JsonObject) -> JsonObject) {
    private var run = submission
    private var bound = false
    fun isAdmitted() = bound
    fun acceptReceipt(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt) {
        require(receipt.identity.sourceHostId == host && receipt.identity.conversationId == conversation
            && receipt.identity.submissionId == submission && receipt.identity.assistantMessageId == "$submission:assistant")
        when (receipt.phase) {
            com.nola.data.remote.api.RemoteSubmissionPhase.Admitted -> {
                run = checkNotNull(receipt.admission).getValue("identity").jsonObject.journalString("run_id"); bound = true
            }
            com.nola.data.remote.api.RemoteSubmissionPhase.Rejected, com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled ->
                error("Summary submission was ${receipt.phase}")
            else -> Unit
        }
    }
    fun observe(envelope: JsonObject) {
        require(bound) { "Summary journal preceded source admission receipt" }
        require(envelope.journalString("source_host_id") == host && envelope.journalString("session_id") == conversation)
        val identity = envelope.getValue("through").jsonObject.getValue("identity").jsonObject
        require(identity.journalString("conversation_id") == conversation)
        require(identity.journalString("run_id") == run)
    }
    private suspend fun query(operation: String, fields: JsonObjectBuilder.() -> Unit = {}): JsonElement {
        val response = remote(buildJsonObject { put("operation", operation); fields() })
        require(response.journalString("source_host_id") == host)
        return response.getValue("result")
    }
    suspend fun terminal(hint: JsonObject? = null): String? {
        if (!bound) return null
        if (hint != null) require(hint.journalString("source_host_id") == host && hint.journalString("session_id") == conversation && (!bound || hint.journalString("run_id") == run))
        val candidate = if (!bound && hint != null) hint.journalString("run_id") else run
        val raw = query("terminal_seal") { put("conversation_id", conversation); put("run_id", candidate) }
        if (raw == JsonNull) return null
        val seal = raw.jsonObject; val identity = seal.getValue("identity").jsonObject
        require(identity.journalString("conversation_id") == conversation && identity.journalString("run_id") == candidate
            && identity.journalString("assistant_message_id") == "$submission:assistant")
        if (hint != null) require(hint["seal_digest"] == seal["seal_digest"])
        run = candidate; bound = true
        require(seal.getValue("protocol_version").jsonPrimitive.content == "1")
        val result = seal.getValue("result").jsonObject
        require(result.journalString("conversation_id") == conversation && result.journalString("run_id") == candidate &&
            result.getValue("value").jsonObject.journalString("kind") == "answered") {
            "Host summary has no committed answer result"
        }
        val total = seal.journalNumber("entry_count"); var after = 0uL; var heads = 0uL
        val text = StringBuilder(); val seen = mutableSetOf<String>()
        while (after < total) {
            val page = query("terminal_manifest") { put("seal", seal); put("after_entry", after.toString()); put("max_records", 64); put("max_encoded_bytes", 131072) }.jsonObject
            val entries = page.getValue("entries").jsonArray; val next = page.journalNumber("next_entry")
            require(page["seal"] == seal && page.journalNumber("after_entry") == after && next > after && next <= total && next - after == entries.size.toULong() && entries.size <= 64)
            require(page.journalBoolean("complete") == (next == total))
            for (entry in entries.map { it.jsonObject }) if (entry.journalString("kind") == "assistant") {
                entry.requireTerminalMessage()
                val through = entry.getValue("through").jsonObject; val owner = through.getValue("identity").jsonObject
                require(owner["source_store_epoch"] == identity["source_store_epoch"] && owner["conversation_id"] == identity["conversation_id"] && owner["run_id"] == identity["run_id"])
                require(seen.add(owner.journalString("assistant_message_id")))
                text.append(readText(through)); heads++
            }
            after = next
        }
        require(heads > 0uL && heads == seal.journalNumber("heads_count") && "$submission:assistant" in seen)
        return text.toString()
    }
    private suspend fun readText(at: JsonObject): String {
        at.journalNumber("sequence")
        val snapshot = query("open_snapshot") { put("at", at) }.jsonObject
        val parts = mutableListOf<Pair<ULong, String>>()
        try {
            require(snapshot["at"] == at)
            var after: String? = null
            do {
                val page = query("parts") { put("snapshot", snapshot); put("after_part_id", after); put("max_records", 64); put("max_encoded_bytes", 262144) }.jsonObject
                require(page["at"] == at && page.getValue("parts").jsonArray.size <= 64)
                for (item in page.getValue("parts").jsonArray) {
                    val head = item.jsonObject; require(head["identity"] == at["identity"])
                    val metadata = head.getValue("metadata").jsonObject; val data = metadata.getValue("data").jsonObject
                    if (metadata.journalString("kind") != "text" || data.journalString("visibility") != "public"
                        || data["review"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content?.let { it != "approved" } == true) continue
                    val content = head.getValue("bodies").jsonArray.map { it.jsonObject }.find { it.journalString("field") == "markdown" } ?: continue
                    val size = content.journalNumber("utf8_bytes"); var offset = 0uL; val text = StringBuilder()
                    while (offset < size) {
                        val page = query("range") { put("snapshot", snapshot); put("part_id", head.getValue("part_id")); put("content", content); put("offset_bytes", offset.toString()); put("limit_bytes", 32768) }.jsonObject
                        val chunk = page.getValue("text").jsonPrimitive.content; val bytes = chunk.toByteArray(Charsets.UTF_8).size; val next = page.journalNumber("next_offset_bytes")
                        require(page.journalNumber("offset_bytes") == offset && page.journalNumber("total_utf8_bytes") == size && bytes in 1..32768
                            && next > offset && next - offset == bytes.toULong() && next <= size && page.journalBoolean("end_of_content") == (next == size))
                        text.append(chunk); offset = next
                    }
                    parts += head.getValue("order_key").jsonObject.journalNumber("position") to text.toString()
                }
                val next = page.getValue("next_part_id").takeUnless { it == JsonNull }?.jsonPrimitive?.content
                after?.let { previous -> require(next == null || next > previous) }; after = next
            } while (after != null)
            return parts.sortedBy { it.first }.joinToString("") { it.second }
        } finally { withContext(NonCancellable) { query("release_snapshot") { put("snapshot", snapshot) } } }
    }
}
