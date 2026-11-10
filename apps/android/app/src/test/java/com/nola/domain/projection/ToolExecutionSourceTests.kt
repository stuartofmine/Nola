package com.nola.domain.projection

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

class ToolExecutionSourceTests {
    private val cell = mapOf(
        "host_id" to "host", "conversation_id" to "conversation", "turn_id" to "turn",
        "parent_call_id" to "parent", "cell_id" to "cell", "generation" to "generation",
    ).mapValues { JsonPrimitive(it.value) }

    private fun source(fields: Map<String, JsonPrimitive> = cell) = JsonObject(mapOf(
        "kind" to JsonPrimitive("code_cell"), "cell" to JsonObject(fields),
    ))

    @Test fun completeIdentitySurvivesDecoding() {
        assertEquals(ToolExecutionSource.CodeCell(CodeCellIdentity(
            "host", "conversation", "turn", "parent", "cell", "generation",
        )), PublicPartSemanticsDecoder.executionSource(source()))
        assertEquals(ToolExecutionSource.Model,
            PublicPartSemanticsDecoder.executionSource(Json.parseToJsonElement("{\"kind\":\"model\"}")))
    }

    @Test fun malformedProvenanceNeverDefaultsToModel() {
        for (raw in listOf("null", "[]", "{}", "{\"kind\":\"future\"}",
            "{\"kind\":\"model\",\"cell\":null}", "{\"kind\":\"model\",\"permission\":\"allow\"}",
            "{\"kind\":\"code_cell\"}")) {
            assertFails { PublicPartSemanticsDecoder.executionSource(Json.parseToJsonElement(raw)) }
        }
        assertFails { PublicPartSemanticsDecoder.executionSource(source(cell + ("alias" to JsonPrimitive("x")))) }
        for (key in cell.keys) {
            assertFails { PublicPartSemanticsDecoder.executionSource(source(cell - key)) }
            for (invalid in listOf("", " ", "owner\n", "\u0085", "界".repeat(171))) {
                assertFails { PublicPartSemanticsDecoder.executionSource(source(cell + (key to JsonPrimitive(invalid)))) }
            }
        }
    }
}
