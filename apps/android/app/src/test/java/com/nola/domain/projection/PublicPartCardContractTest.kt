package com.nola.domain.projection

import kotlinx.serialization.json.*
import kotlin.test.*

class PublicPartCardContractTest {
    private fun decode(raw: String, owner: String = "message-1") =
        ConversationProjectionV1Decoder.decodeDisplayPart(Json.parseToJsonElement(raw).jsonObject, owner)

    @Test fun richToolRetainsNullMissingPreviewProvenancePermissionAndExactUnsignedTime() {
        val wire = """{"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":1,"tool_call_id":"call","capability_id":"file.read",
            "title":"Human title","summary":"","state":"completed","origin":{"kind":"mcp","provider_id":"p","provider_title":"Provider","namespace":"ns"},
            "input":null,"output":null,"output_preview":"technical",
            "full_output":{"message_id":"message-1","part_id":"tool","tool_call_id":"call"},
            "permission":{"request_id":"approval","permission_domain":"filesystem","rationale":"Requested file","decision":"allow"},
            "artifacts":[{"uri":"file:///tmp/report.pdf","name":"report.pdf","mime_type":"application/pdf","operation":"write"}],
            "affected_resources":[],"started_at_ms":0,"finished_at_ms":18446744073709551615}"""
        val tool = decode(wire) as ConversationProjectionV1.Part.Tool
        val facts = tool.cardFacts()
        assertEquals(JsonNull, tool.input); assertEquals(JsonNull, facts.result)
        assertEquals(ULong.MAX_VALUE, tool.finishedAtMs)
        assertEquals("Human title", facts.title); assertEquals("", facts.summary)
        assertEquals("technical", facts.preview); assertEquals("approval", facts.permission?.requestId)
        assertEquals("Provider", facts.origin.providerTitle); assertEquals("write", facts.artifacts.single().operation)
        assertEquals(ToolOutputOwner("message-1", "tool", "call"), facts.fullOutput)
        val absent = decode(wire.replace("\"output\":null,", "")) as ConversationProjectionV1.Part.Tool
        assertNull(absent.output); assertEquals("technical", absent.cardFacts().preview)
        assertFails { decode(wire, "other-owner") }
        assertFails { decode(wire.replace("\"input\":null,", "")) }
        assertFails { decode(wire.replace("\"summary\":\"\"", "\"summary\":42")) }
        assertFails { decode(wire.replace("\"rationale\":\"Requested file\"", "\"rationale\":\"Requested file\",\"grant\":\"private\"")) }
        assertFails { decode(wire.replace("\"message_id\":\"message-1\"", "\"message_id\":\"message-1\",\"path\":\"/private/output\"")) }
    }

    @Test fun typedSourceMemoryAndCitationPreserveIndependentIdsAndUnsignedRanges() {
        val source = decode("""{"kind":"source","part_id":"source-part","revision":1,"source_id":"memory-2",
            "title":"Workspace preference","uri":"memory://memory-2","evidence":{"kind":"memory","usage":{
            "record_id":"memory-2","component_id":"context-memory-2","scope":"global","namespace":{"kind":"workspace"},
            "provenance_kind":"explicit_user","source":"conversation://session-1/message-9","observed_at":"2026-08-06T00:00:00Z",
            "requires_current_state_verification":true,"truncated":false,"supersedes":["memory-1"],"usage_count":4,"last_used_at_epoch_ms":1786000000000}}}""") as ConversationProjectionV1.Part.Source
        val memory = source.evidence?.usage ?: error("Missing typed memory evidence")
        assertEquals("memory-2", memory.recordId); assertEquals(MemoryNamespace("workspace"), memory.namespace)
        assertEquals(listOf("memory-1"), memory.supersedes); assertEquals(4uL, memory.usageCount)
        assertTrue(memory.requiresCurrentStateVerification)
        val citation = decode("""{"kind":"citation","part_id":"citation-part","revision":1,"citation_id":"actual-citation","source_id":"memory-2",
            "title":"Citation","uri":"file:///tmp/report.pdf","mime_type":"application/pdf","origin_tool_call_id":"call",
            "location":{"page":18446744073709551615,"line_start":0,"line_end":18446744073709551615,"section":"Section"},
            "page":18446744073709551615,"line_start":0,"line_end":18446744073709551615,"section":"Section",
            "range_start":0,"range_end":18446744073709551615}""") as ConversationProjectionV1.Part.Citation
        assertEquals("actual-citation", citation.citationId); assertEquals(source.sourceId, citation.sourceId)
        assertEquals("call", citation.originToolCallId); assertEquals(ULong.MAX_VALUE, citation.page)
        assertEquals(CitationRange(0uL, ULong.MAX_VALUE), citation.range)
    }
}
