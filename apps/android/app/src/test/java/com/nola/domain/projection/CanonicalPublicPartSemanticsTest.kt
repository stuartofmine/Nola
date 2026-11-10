package com.nola.domain.projection

import com.nola.domain.model.decodeAnswerSourcesFromMeta
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject

class CanonicalPublicPartSemanticsTest {
    private fun shared(): JsonObject = Json.parseToJsonElement(checkNotNull(
        javaClass.classLoader?.getResourceAsStream("conversation_projection_v1.json"),
    ).bufferedReader().use { it.readText() }).jsonObject

    private fun withParts(parts: List<JsonElement>): JsonObject {
        val root = shared()
        val assistant = root.getValue("messages").jsonArray[1].jsonObject
        return JsonObject(root + ("messages" to JsonArray(listOf(JsonObject(assistant + ("parts" to JsonArray(parts)))))))
    }

    private fun tool(): JsonObject = shared().getValue("messages").jsonArray[1].jsonObject.getValue("parts").jsonArray[1].jsonObject
    private fun projected(parts: List<JsonElement>) = CanonicalConversationProjector.project(ConversationProjectionV1Decoder.decode(withParts(parts))).single()

    @Test
    fun sharedRealContractFlowsThroughProjectorIntoTheSameFactsConsumedByCompose() {
        val projection = ConversationProjectionV1Decoder.decode(shared())
        val message = CanonicalConversationProjector.project(projection)[1]
        val reasoning = message.canonicalParts[0] as ConversationProjectionV1.Part.Reasoning
        val facts = (message.canonicalParts[1] as ConversationProjectionV1.Part.Tool).cardFacts()
        assertEquals("Inspect evidence", reasoning.title)
        assertEquals("Read architecture", facts.title)
        assertEquals("Read the contract", facts.summary)
        assertEquals(Json.parseToJsonElement("""{"text":"Architecture contract"}"""), facts.result)
        assertEquals("Independent technical preview", facts.preview)
        assertEquals("builtin", facts.origin.kind)
        assertEquals("read", facts.artifacts.single().operation)
        assertEquals(1787443201000uL, facts.startedAtMs)
        assertEquals(1787443201010uL, facts.finishedAtMs)
    }

    @Test
    fun nullInputAndAbsentNullEmptyAndLongUnicodeResultsRemainIndependent() {
        val longUnicode = "界🙂".repeat(20000)
        val results = listOf(null, JsonNull, JsonPrimitive(""), JsonArray(emptyList()), JsonObject(emptyMap()), JsonObject(mapOf("text" to JsonPrimitive(longUnicode))))
        results.forEach { result ->
            val values = tool().toMutableMap().also {
                it["input"] = JsonNull
                it["summary"] = JsonPrimitive("")
                it["output_preview"] = JsonPrimitive("technical-only")
                it["title"] = JsonPrimitive(longUnicode)
                it.remove("output")
                if (result != null) it["output"] = result
                it["started_at_ms"] = Json.parseToJsonElement("0")
                it["finished_at_ms"] = Json.parseToJsonElement(ULong.MAX_VALUE.toString())
                it["full_output"] = Json.parseToJsonElement("""{"message_id":"contract-assistant-1","part_id":"contract-assistant-1:tool:call-1","tool_call_id":"call-1"}""")
            }
            val facts = (projected(listOf(JsonObject(values))).canonicalParts.single() as ConversationProjectionV1.Part.Tool).cardFacts()
            assertEquals(JsonNull, facts.input)
            assertEquals(result, facts.result)
            assertEquals("", facts.summary)
            assertEquals("technical-only", facts.preview)
            assertEquals(longUnicode, facts.title)
            assertEquals(0uL, facts.startedAtMs)
            assertEquals(ULong.MAX_VALUE, facts.finishedAtMs)
            assertEquals(ToolOutputOwner("contract-assistant-1", "contract-assistant-1:tool:call-1", "call-1"), facts.fullOutput)
        }
    }

    @Test
    fun sourceEvidenceAndCitationIdentityLocationRangeAndAttachmentAreNotFlattened() {
        val source = Json.parseToJsonElement("""{
            "kind":"source","part_id":"source-part","revision":7,"source_id":"source","title":"Evidence","uri":"memory://source",
            "mime_type":"text/plain","snippet":"snippet","evidence":{"kind":"memory","usage":{
                "record_id":"record","component_id":"component","scope":"turn","namespace":{"kind":"custom","name":"workspace-not-kind"},
                "provenance_kind":"tool_result","source":"source-uri","title":"included title","observed_at":"observed","source_role":"tool",
                "event_time":"event","range_start":"start","range_end":"end","requires_current_state_verification":true,"truncated":true,
                "supersedes":["older"],"usage_count":18446744073709551615,"last_used_at_epoch_ms":-9223372036854775808
            }}
        }""")
        fun citation(id: String, location: String) = Json.parseToJsonElement("""{
            "kind":"citation","part_id":"part-$id","revision":7,"citation_id":"real-$id","source_id":"source","title":"Same URI","uri":"https://example.com/report",
            "mime_type":"application/pdf","origin_tool_call_id":"call","location":$location,"section":null,"page":null,"line_start":null,"line_end":null,
            "range_start":0,"range_end":18446744073709551615
        }""")
        val attachment = Json.parseToJsonElement("""{"kind":"attachment","part_id":"artifact-part","revision":7,"attachment_id":"attachment","name":"report.pdf","mime_type":"application/pdf","uri":"file:///tmp/report.pdf","operation":"write"}""")
        val message = projected(listOf(source, citation("one", "null"), citation("two", "{}"), attachment))
        val facts = message.canonicalParts[0].sourceCardFacts()!!
        val evidence = facts.evidence!!.usage
        assertEquals(MemoryNamespace("custom", "workspace-not-kind"), evidence.namespace)
        assertEquals(ULong.MAX_VALUE, evidence.usageCount)
        assertEquals(Long.MIN_VALUE, evidence.lastUsedAtEpochMs)
        assertEquals("tool", evidence.sourceRole)
        assertEquals("event", evidence.eventTime)
        assertEquals("start", evidence.rangeStart)
        assertEquals("end", evidence.rangeEnd)
        assertEquals(listOf("older"), evidence.supersedes)
        assertTrue(evidence.truncated && evidence.requiresCurrentStateVerification)
        val first = message.canonicalParts[1].sourceCardFacts()!!
        val second = message.canonicalParts[2].sourceCardFacts()!!
        assertEquals("real-one", first.identity)
        assertEquals("application/pdf", first.mimeType)
        assertNull(first.location)
        assertEquals(SourceLocation(), second.location)
        assertEquals(CitationRange(0uL, ULong.MAX_VALUE), first.range)
        assertEquals(listOf("source-part", "real-one", "real-two"), decodeAnswerSourcesFromMeta(message.meta).map { it.id })
        assertEquals("write", (message.canonicalParts[3] as ConversationProjectionV1.Part.Attachment).operation)
        val coordinateCitation = (citation("coordinate", "{}") as JsonObject).toMutableMap().also {
            it["location"] = Json.parseToJsonElement("""{"page":18446744073709551615,"line_start":0,"line_end":18446744073709551615,"section":"结果🙂"}""")
            it["page"] = Json.parseToJsonElement(ULong.MAX_VALUE.toString())
            it["line_start"] = Json.parseToJsonElement("0")
            it["line_end"] = Json.parseToJsonElement(ULong.MAX_VALUE.toString())
            it["section"] = JsonPrimitive("结果🙂")
        }
        val coordinates = projected(listOf(JsonObject(coordinateCitation))).canonicalParts.single().sourceCardFacts()!!
        assertEquals(SourceLocation(ULong.MAX_VALUE, 0uL, ULong.MAX_VALUE, "结果🙂"), coordinates.location)
        assertNull(ULong.MAX_VALUE.asLegacyLongOrNull())
        assertEquals(Long.MAX_VALUE, Long.MAX_VALUE.toULong().asLegacyLongOrNull())
    }

    @Test
    fun reviewOwnerAndReadonlyNestedBoundariesRejectLeakage() {
        for (review in listOf("pending", "rejected", "interrupted")) {
            val hidden = Json.parseToJsonElement("""{"kind":"text","part_id":"text","revision":7,"markdown":"","is_final":true,"review":"$review"}""").jsonObject
            val part = projected(listOf(hidden)).canonicalParts.single() as ConversationProjectionV1.Part.Text
            assertEquals("", part.disclosedMarkdown)
            assertFails { projected(listOf(JsonObject(hidden + ("markdown" to JsonPrimitive("private response"))))) }
        }
        for (field in listOf("full_output_ref", "raw_input", "grant")) {
            assertFails { projected(listOf(JsonObject(tool() + (field to JsonPrimitive("private"))))) }
        }
        for (extra in listOf("url", "path", "grant")) {
            val owner = JsonObject(mapOf("message_id" to JsonPrimitive("contract-assistant-1"), "part_id" to JsonPrimitive("contract-assistant-1:tool:call-1"), "tool_call_id" to JsonPrimitive("call-1"), extra to JsonPrimitive("private")))
            assertFails { projected(listOf(JsonObject(tool() + ("full_output" to owner)))) }
        }
        assertFails { projected(listOf(JsonObject(tool() + ("full_output" to Json.parseToJsonElement("""{"message_id":"foreign","part_id":"contract-assistant-1:tool:call-1","tool_call_id":"call-1"}"""))))) }
        assertFails { PublicPartSemanticsDecoder.evidence(Json.parseToJsonElement("""{"kind":"memory","usage":{"record_id":"r","component_id":"c","body":"private memory"}}""")) }
        assertFails { PublicPartSemanticsDecoder.evidence(Json.parseToJsonElement("""{"kind":"memory","usage":{"record_id":"r","component_id":"c","namespace":{"kind":"workspace","name":"ignored"}}}""")) }
        assertFails { PublicPartSemanticsDecoder.permission(Json.parseToJsonElement("""{"request_id":"r","permission_domain":"file","rationale":"why","lease":"private"}""")) }
        assertFails { PublicPartSemanticsDecoder.unsigned(JsonPrimitive("18446744073709551615")) }
        assertFails { PublicPartSemanticsDecoder.unsigned(Json.parseToJsonElement("18446744073709551616")) }
        val approved = Json.parseToJsonElement("""{"kind":"text","part_id":"approved","revision":7,"markdown":"approved answer","is_final":true,"review":"approved"}""")
        assertEquals("approved answer", projected(listOf(approved)).text)
        for (field in listOf("message_id", "part_id", "tool_call_id")) {
            val owner = Json.parseToJsonElement("""{"message_id":"contract-assistant-1","part_id":"contract-assistant-1:tool:call-1","tool_call_id":"call-1"}""").jsonObject
            assertFails { projected(listOf(JsonObject(tool() + ("full_output" to JsonObject(owner + (field to JsonPrimitive("foreign"))))))) }
        }
        val incomplete = Json.parseToJsonElement("""{"kind":"citation","part_id":"p","revision":7,"citation_id":"real","source_id":"source","title":"Citation","uri":"https://example.com","origin_tool_call_id":"call","location":{},"range_start":1}""")
        assertFails { projected(listOf(incomplete)) }
        val mismatched = JsonObject(incomplete.jsonObject + ("range_end" to JsonPrimitive(2)) + ("page" to JsonPrimitive(3)))
        assertFails { projected(listOf(mismatched)) }
    }

    @Test
    fun defaultsAreMissingOnlyAndDisplayStringsOrUnsignedFactsCannotBeCoerced() {
        assertEquals(CapabilityOrigin(), PublicPartSemanticsDecoder.origin(JsonObject(emptyMap())))
        assertFails { PublicPartSemanticsDecoder.origin(Json.parseToJsonElement("""{"kind":null}""")) }
        assertFails { PublicPartSemanticsDecoder.origin(Json.parseToJsonElement("""{"kind":"builtin","provider_title":42}""")) }
        assertFails { PublicPartSemanticsDecoder.permission(Json.parseToJsonElement("""{"request_id":"r","permission_domain":"file","rationale":42}""")) }
        assertFails { PublicPartSemanticsDecoder.location(Json.parseToJsonElement("""{"section":true}""")) }
        val usage = Json.parseToJsonElement("""{"record_id":"r","component_id":"c"}""").jsonObject
        val defaultEvidence = JsonObject(mapOf("kind" to JsonPrimitive("memory"), "usage" to usage))
        assertEquals(0uL, PublicPartSemanticsDecoder.evidence(defaultEvidence)?.usage?.usageCount)
        for (field in listOf("usage_count", "requires_current_state_verification", "truncated", "supersedes")) {
            assertFails { PublicPartSemanticsDecoder.evidence(JsonObject(defaultEvidence + ("usage" to JsonObject(usage + (field to JsonNull))))) }
        }
        for (field in listOf("title", "summary", "output_preview")) {
            assertFails { projected(listOf(JsonObject(tool() + (field to JsonPrimitive(42))))) }
        }
        assertFails { projected(listOf(JsonObject(tool() - "input"))) }
        for (number in listOf("-1", "1.0", "1e3", "true")) {
            assertFails { PublicPartSemanticsDecoder.unsigned(Json.parseToJsonElement(number)) }
        }
        assertFails { PublicPartSemanticsDecoder.unsigned(JsonPrimitive("1")) }
    }
}
