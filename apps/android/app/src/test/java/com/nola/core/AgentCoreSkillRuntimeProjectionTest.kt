package com.nola.core

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreSkillRuntimeProjectionTest {
    @Test
    fun sharedCapabilityActivationContractDecodesCoreEnvelope() {
        val fixtureFile = generateSequence(
            java.io.File(requireNotNull(System.getProperty("user.dir"))),
        ) { it.parentFile }
            .map { root -> java.io.File(root, "core/tests/fixtures/capability_activation_v1.json") }
            .firstOrNull(java.io.File::isFile)
            ?: error("capability activation fixture is missing")
        val fixture = kotlinx.serialization.json.Json.parseToJsonElement(fixtureFile.readText())
            as kotlinx.serialization.json.JsonObject
        val envelope = fixture["core_envelope"] as kotlinx.serialization.json.JsonObject
        val activation = requireNotNull(decodeAgentCoreCapabilityActivationEnvelope(envelope))

        assertEquals("conversation-a", activation.sessionId)
        assertEquals("turn-a", activation.turnId)
        assertEquals(7, activation.cursor)
        assertEquals(11, activation.journalRevision)
        assertEquals("tool_search", activation.source)
        assertEquals(listOf("web_search", "web_fetch"), activation.capabilityIds)
        assertEquals(
            "core-capability:turn-a:tool_search",
            activation.toHostToolEvent().id,
        )
    }

    @Test
    fun authoritativeLifecycleIsMonotonicAndMapsToStableToolEvent() {
        val started = decodeAgentCoreSkillRuntimeEnvelope(envelope(1, 1, 1, startedUpdate()))!!
        val first = AgentCoreSkillRuntimeProjectionReducer.reduce(
            AgentCoreSkillRuntimeProjection(),
            started,
            expectedSessionId = "session-1",
            expectedTurnId = "turn-1",
        )
        assertTrue(first.applied)
        assertEquals("core-skill:run-1", first.projection.mergeIntoToolEvents(emptyList()).single().id)

        val completed = decodeAgentCoreSkillRuntimeEnvelope(
            envelope(2, 2, 2, terminalUpdate("completed")),
        )!!
        val second = AgentCoreSkillRuntimeProjectionReducer.reduce(
            first.projection,
            completed,
            expectedSessionId = "session-1",
            expectedTurnId = "turn-1",
        )
        assertTrue(second.applied)
        assertEquals("completed", second.projection.mergeIntoToolEvents(emptyList()).single().status)
        assertFalse(
            AgentCoreSkillRuntimeProjectionReducer.reduce(
                second.projection,
                decodeAgentCoreSkillRuntimeEnvelope(envelope(3, 3, 3, terminalUpdate("failed")))!!,
                "session-1",
            ).applied,
        )
    }

    @Test
    fun decoderRejectsTextAndWrongOwnerAndRestoresCheckpoint() {
        assertNull(decodeAgentCoreSkillRuntimeEnvelope(buildJsonObject { put("text", "skill_runtime_updated") }))
        val wrongOwner = decodeAgentCoreSkillRuntimeEnvelope(
            envelope(1, 1, 1, startedUpdate(ownerSession = "other")),
        )!!
        assertFalse(
            AgentCoreSkillRuntimeProjectionReducer.reduce(
                AgentCoreSkillRuntimeProjection(),
                wrongOwner,
                "session-1",
            ).applied,
        )

        val checkpoint = buildJsonObject {
            put("revision", 4)
            put("catalog", buildJsonObject { put("revision", 3) })
            put("runs", buildJsonObject { put("run-1", skillRun()) })
        }
        val restored = decodeAgentCoreSkillRuntimeProjection(checkpoint, "session-1")
        assertNotNull(restored)
        assertEquals(4L, restored?.revision)
        assertEquals("run-1", restored?.orderedRuns?.single()?.runId)
    }

    @Test
    fun currentSkillEventsRetainHostEvidenceAndRejectModelSelfPromotion() {
        var projection = AgentCoreSkillRuntimeProjection()
        val updates = listOf(startedUpdate(),
            Json.parseToJsonElement("""{"type":"progress_updated","run_id":"run-1","step_id":"draft","completed":true}""") as JsonObject,
            Json.parseToJsonElement("""{"type":"artifact_registered","run_id":"run-1","artifact_id":"report","artifact_ref":"artifact://report"}""") as JsonObject,
            Json.parseToJsonElement("""{"type":"verification_recorded","run_id":"run-1","result":{"verifierId":"check","passed":true,"evidence":{"kind":"host_check","source":{"kind":"tool_call","call_id":"write-1"},"resource_uri":"file:///fixture/report","version":"sha256:original","check_kind":"postcondition","predicate":"content_matches"},"replacesEvidence":null,"summary":"Host receipt"}}""") as JsonObject,
            Json.parseToJsonElement("""{"type":"resume_point_updated","run_id":"run-1","resume_point":"review"}""") as JsonObject)
        updates.forEachIndexed { index, update ->
            val n = index.toLong() + 1
            val reduction = AgentCoreSkillRuntimeProjectionReducer.reduce(projection,
                requireNotNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(n, n, n, update))), "session-1", "turn-1")
            assertTrue(reduction.applied)
            projection = reduction.projection
        }
        val run = projection.runs.getValue("run-1")
        assertEquals(setOf("draft"), run.completedProgressSteps)
        assertNull(run.currentProgressStep)
        assertEquals("artifact://report", run.artifacts["report"])
        assertEquals("host_check", run.verifications["check"]?.evidence?.kind)
        assertEquals("write-1", run.verifications["check"]?.evidence?.sources?.single()?.id)
        assertEquals("sha256:original", run.verifications["check"]?.evidence?.version)
        assertEquals("review", run.resumePoint)
        val snapshotRun = JsonObject(skillRun() + mapOf(
            "completed_progress_steps" to Json.parseToJsonElement("""["draft"]"""),
            "artifacts" to Json.parseToJsonElement("""{"report":"artifact://report"}"""),
            "verifications" to buildJsonObject { put("check", updates[3]["result"]!!) },
            "resume_point" to kotlinx.serialization.json.JsonPrimitive("review")))
        val restored = requireNotNull(decodeAgentCoreSkillRuntimeProjection(buildJsonObject {
            put("revision", 5); put("runs", buildJsonObject { put("run-1", snapshotRun) })
        }, "session-1"))
        assertEquals(run.copy(currentProgressStep = null), restored.runs["run-1"])
        val selfClaim = Json.parseToJsonElement("""{"type":"verification_recorded","run_id":"run-1","result":{"verifierId":"check","passed":true,"evidence":{"kind":"model_judgment","sources":[]},"replacesEvidence":null,"summary":"I checked"}}""") as JsonObject
        assertFalse(AgentCoreSkillRuntimeProjectionReducer.reduce(projection,
            requireNotNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(6, 6, 6, selfClaim))), "session-1").applied)
        val changedSource = Json.parseToJsonElement(updates[3].toString().replace("write-1", "write-2")) as JsonObject
        assertFalse(AgentCoreSkillRuntimeProjectionReducer.reduce(projection,
            requireNotNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(6, 6, 6, changedSource))), "session-1").applied)
        val oldEvidence = (updates[3]["result"] as JsonObject)["evidence"]!!
        val replacement = JsonObject(changedSource + ("result" to JsonObject(
            (changedSource["result"] as JsonObject) + ("replacesEvidence" to oldEvidence))))
        val replaced = AgentCoreSkillRuntimeProjectionReducer.reduce(projection,
            requireNotNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(6, 6, 6, replacement))), "session-1")
        assertTrue(replaced.applied)
        assertEquals("write-2", replaced.projection.runs["run-1"]?.verifications?.get("check")?.evidence?.sources?.single()?.id)
        val malformed = Json.parseToJsonElement("""{"type":"verification_recorded","run_id":"run-1","result":{"verifierId":"check","passed":true,"evidence":{"kind":"host_check","sources":[]},"summary":"I checked"}}""") as JsonObject
        assertNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(6, 6, 6, malformed)))
        val descriptor = skillRun()["descriptor"] as JsonObject
        val missingDigest = JsonObject(skillRun() + ("descriptor" to JsonObject(descriptor - "instructions_digest")))
        assertNull(decodeAgentCoreSkillRuntimeEnvelope(envelope(1, 1, 1, buildJsonObject {
            put("type", "started"); put("run", missingDigest)
        })))
    }

    private fun envelope(cursor: Long, revision: Long, skillRevision: Long, update: kotlinx.serialization.json.JsonObject) =
        buildJsonObject {
            put("session_id", "session-1")
            put("turn_id", "turn-1")
            put("revision", revision)
            put("cursor", buildJsonObject { put("sequence", cursor) })
            put("event", buildJsonObject {
                put("type", "skill_runtime_updated")
                put("turn_id", "turn-1")
                put("skill_runtime_revision", skillRevision)
                put("update", update)
            })
        }

    private fun startedUpdate(ownerSession: String = "session-1") = buildJsonObject {
        put("type", "started")
        put("run", skillRun(ownerSession))
    }

    private fun terminalUpdate(type: String) = buildJsonObject {
        put("type", type)
        put("run_id", "run-1")
        if (type == "completed") put("result_ref", "artifact://report.docx")
        if (type == "failed") put("error", "late")
    }

    private fun skillRun(ownerSession: String = "session-1") = buildJsonObject {
        put("run_id", "run-1")
        put("owner", buildJsonObject {
            put("session_id", ownerSession)
            put("parent_turn_id", "turn-1")
        })
        put("descriptor", Json.parseToJsonElement("""{"skill_id":"documents","version":"1","display_name":"Word document","instructions_ref":"builtin://documents#sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","instructions_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","execution_contract":{"schemaVersion":1,"progressSteps":[{"id":"draft","title":"Draft","required":true}],"artifacts":[{"id":"report","mediaType":"text/plain","required":true,"description":"Report"}],"verifiers":[{"id":"check","description":"Check bytes","method":"host_check","required":true}],"resumePoints":["review"]}}"""))
        put("mode", "inline")
        put("status", "running")
        put("usage", buildJsonObject {
            put("model_turns", 0)
            put("tool_calls", 0)
            put("elapsed_millis", 0)
        })
        put("catalog_revision", 1)
        put("completed_progress_steps", kotlinx.serialization.json.JsonArray(emptyList()))
        put("artifacts", buildJsonObject {})
        put("verifications", buildJsonObject {})
    }
}
