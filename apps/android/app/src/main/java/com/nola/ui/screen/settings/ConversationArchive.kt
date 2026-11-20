// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import android.util.JsonReader
import android.util.JsonToken
import com.nola.data.local.AndroidApplicationRuntime
import java.io.InputStream
import java.io.SequenceInputStream
import java.util.Enumeration
import kotlinx.serialization.json.*
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** Human-readable transcripts. IDs, connection settings and tool payloads stay on the device. */
internal fun writeConversationDocuments(zip: ZipOutputStream, runtime: AndroidApplicationRuntime) {
    fun text(value: String) = zip.write(value.toByteArray(Charsets.UTF_8))
    for ((index, summary) in runtime.listConversations().withIndex()) {
        fun loadPage(after: Long?, revision: Long?): JsonObject = runtime.executeQuery(buildJsonObject {
            put("type", "load_conversation_export_page")
            put("conversation_id", summary.id)
            after?.let { put("after_ordinal", it) }
            revision?.let { put("expected_revision", it) }
        }).jsonObject

        var page = loadPage(null, null)
        val header = page.getValue("conversation").jsonObject
        val revision = header.getValue("revision").jsonPrimitive.long
        check(header.getValue("id").jsonPrimitive.content == summary.id)
        zip.putNextEntry(ZipEntry("conversations/${index + 1}.md"))
        text("# " + header.getValue("title").jsonPrimitive.content + "\n")
        var lastOrdinal: Long? = null
        while (true) {
            check(page.getValue("conversation") == header) { "Conversation changed during export" }
            val messages = page.getValue("messages").jsonArray
            for (element in messages) {
                val message = element.jsonObject
                val ordinal = message.getValue("ordinal").jsonPrimitive.long
                check(lastOrdinal == null || ordinal > lastOrdinal)
                lastOrdinal = ordinal
                val role = message.getValue("role").jsonPrimitive.content
                if (role !in setOf("user", "assistant")) continue
                val messageId = message.getValue("id").jsonPrimitive.content
                for (item in message.getValue("parts").jsonArray) {
                    val part = item.jsonObject
                    if (part.getValue("kind").jsonPrimitive.content != "text") continue
                    val partId = part.getValue("id").jsonPrimitive.content
                    val total = part.getValue("content_text_utf8_bytes").jsonPrimitive.long
                    val fieldReader = ArchiveBody(runtime, summary.id, messageId, partId, revision)
                    if (total > 0) {
                        text("\n## $role\n\n")
                        fieldReader.chunks("content_text", total).forEach(zip::write)
                        text("\n")
                    } else {
                        val jsonBytes = part["content_json_utf8_bytes"]?.jsonPrimitive?.longOrNull ?: continue
                        if (jsonBytes > 0) fieldReader.visibleTypedText(jsonBytes)?.let { visible ->
                            text("\n## $role\n\n")
                            text(visible)
                            text("\n")
                        }
                    }
                }
            }
            if (!page.getValue("has_more").jsonPrimitive.boolean) break
            check(messages.isNotEmpty() && page.getValue("next_ordinal").jsonPrimitive.long == lastOrdinal)
            page = loadPage(lastOrdinal, revision)
        }
        check(loadPage(null, revision).getValue("conversation") == header)
        zip.closeEntry()
    }
}


private class ArchiveBody(
    val runtime: AndroidApplicationRuntime,
    val conversation: String,
    val message: String,
    val part: String,
    val revision: Long,
) {
    fun chunks(field: String, total: Long): Sequence<ByteArray> = sequence {
        var offset = 0L
        while (offset < total) {
            val range = runtime.executeQuery(buildJsonObject {
                put("type", "read_conversation_export_range")
                put("conversation_id", conversation)
                put("message_id", message)
                put("part_id", part)
                put("field", field)
                put("expected_revision", revision)
                put("offset_bytes", offset)
                put("limit_bytes", 32 * 1024)
            }).jsonObject
            val bytes = range.getValue("text").jsonPrimitive.content.toByteArray(Charsets.UTF_8)
            val next = range.getValue("next_offset_bytes").jsonPrimitive.long
            check(range.getValue("revision").jsonPrimitive.long == revision &&
                range.getValue("conversation_id").jsonPrimitive.content == conversation &&
                range.getValue("message_id").jsonPrimitive.content == message &&
                range.getValue("part_id").jsonPrimitive.content == part &&
                range.getValue("field").jsonPrimitive.content == field &&
                range.getValue("total_utf8_bytes").jsonPrimitive.long == total &&
                range.getValue("offset_bytes").jsonPrimitive.long == offset &&
                bytes.isNotEmpty() && next == offset + bytes.size && next <= total &&
                range.getValue("end_of_content").jsonPrimitive.boolean == (next == total))
            yield(bytes)
            offset = next
        }
    }

    fun visibleTypedText(length: Long): String? {
        val pieces = chunks("content_json", length).iterator()
        val input = SequenceInputStream(object : Enumeration<InputStream> {
            override fun hasMoreElements() = pieces.hasNext()
            override fun nextElement(): InputStream = pieces.next().inputStream()
        })
        return JsonReader(input.reader(Charsets.UTF_8)).use { json ->
            if (json.peek() != JsonToken.BEGIN_OBJECT) return@use null
            json.beginObject()
            var kind: String? = null
            var visible: String? = null
            while (json.hasNext()) {
                when (json.nextName()) {
                    "kind" -> if (json.peek() == JsonToken.STRING) kind = json.nextString() else json.skipValue()
                    "text" -> if (json.peek() == JsonToken.STRING) visible = json.nextString() else json.skipValue()
                    else -> json.skipValue()
                }
            }
            json.endObject()
            visible?.takeIf { kind == "text" && it.isNotEmpty() }
        }
    }
}
