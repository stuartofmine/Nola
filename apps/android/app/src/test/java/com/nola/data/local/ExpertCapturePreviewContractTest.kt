package com.nola.data.local

import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertCapturePreviewContractTest {
    private fun participant() = buildJsonObject {
        put("participant", buildJsonObject {
            put("participant_id", "participant"); put("agent_id", "expert")
            put("agent_revision", 1); put("context_conversation_id", "context")
        })
        put("context_configuration_revision", 2)
        put("defaults", buildJsonObject {})
        put("model_declaration", buildJsonObject { put("runtime", "mlx") })
        put("client_model_presentation", JsonNull)
        put("effective_parameters", buildJsonObject {
            put("status", "unavailable"); put("reason", "local_baseline_unavailable")
        })
    }

    @Test fun currentPreviewPreservesAuthoritativeUnavailableInsteadOfClientDefaults() {
        val entry = participant()
        val preview = Json.decodeFromJsonElement<AndroidExpertCapturePreview>(buildJsonObject {
            put("participants", JsonArray(listOf(entry)))
        })
        assertEquals(entry.getValue("effective_parameters"), preview.participants.single().effectiveParameters)
        assertEquals(JsonNull, preview.participants.single().clientModelPresentation)
    }

    @Test fun missingOrNullEffectiveParametersAreRejected() {
        val entry = participant()
        for (invalid in listOf(JsonObject(entry.filterKeys { it != "effective_parameters" }),
            JsonObject(entry + ("effective_parameters" to JsonNull)))) {
            assertThrows(SerializationException::class.java) {
                Json.decodeFromJsonElement<AndroidExpertCapturePreviewEntry>(invalid)
            }
        }
    }
}
