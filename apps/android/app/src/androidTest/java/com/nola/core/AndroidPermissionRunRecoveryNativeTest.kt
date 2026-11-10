package com.nola.core

import androidx.test.core.app.ApplicationProvider
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.repository.ApiModelConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.llm.provider.OpenAiProvider
import io.ktor.client.HttpClient
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** Real JNI persistence and Host protocol. Model events are scripted, not a real-provider acceptance. */
class AndroidPermissionRunRecoveryNativeTest {
    @Test fun committedPermissionWaitReopensSameRunAndProducesJournalTerminalSeal() = runBlocking {
        assertTrue("Real Rust JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "permission-run-native-${System.nanoTime()}").also { it.mkdirs() }
        val database = File(root, "isolated.db")
        val http = HttpClient()
        val provider = OpenAiProvider(http)
        val endpoint = ApiModelConfig("https://fixture.invalid/v1", "test-only-credential", "original-model")
        val model = ModelInfo("original-model", "Original")
        val parameters = ModelParameters(maxCompletionTokens = 1024, maxContextTokens = 8192)
        val captured = cloudTurnModelSnapshot(model, parameters, endpoint, provider)
        val conversation = "conversation-route"
        val run = "original-run"
        val initialOwner = "opaque-initial-owner"
        var binding: JsonObject? = null
        var originalDelivery: JsonObject? = null
        try {
            AndroidApplicationRuntime(context, database).use { store ->
                val route = requireNotNull(captured.route)
                val admission = store.execute(buildJsonObject {
                    put("type", "admit_host_output")
                    put("routes", buildJsonArray { add(buildJsonObject {
                        put("route", route); put("max_input_tokens", JsonNull)
                        put("max_output_tokens", JsonNull); put("max_total_tokens", JsonNull)
                    }) })
                    put("preference", buildJsonObject { put("explicit", 1024) })
                })
                binding = buildJsonObject {
                    put("version", 7); put("primary", route); put("fallbacks", JsonArray(emptyList()))
                    put("generation", captured.invocationGeneration(true)); put("output_admission", admission)
                    put("execution_policy_sha256", invocationSha256("fixture-read-only-tool-contract"))
                }
                val runtime = AndroidCanonicalAgentRuntime(store,
                    AndroidCanonicalTurnDescriptor(conversation, "Native recovery", run, "user-original", "Read fixture", null,
                        assistantMessageId = initialOwner), effectLeaseMilliseconds = 2_000)
                runtime.configure(config(requireNotNull(binding)))
                var transition = runtime.submit(json("""{"type":"user_input","submission_id":"original-run","delivery":"current_turn","submitted_at_epoch_ms":1,"items":[{"type":"message","id":"user-original","role":"user","content":"Read fixture"}]}""")).getValue("transition").jsonObject
                val driver = Driver(store, runtime, run)
                val request = driver.request(transition, "model_stream")
                driver.respond(request, json("""{"type":"execution_started"}"""))
                driver.respond(request, json("""{"type":"model_attempt_started","provider":"fixture","model":"original-model","reason":"initial"}"""))
                transition = driver.respond(request, json("""{"type":"model_completed","items":[],"tool_calls":[{"call_id":"call-read","name":"fixture_read","arguments":{}}],"usage":{"input_tokens":8,"output_tokens":3,"total_tokens":11}}"""))
                val waitingPermission = driver.request(transition, "request_permission")
                originalDelivery = waitingPermission.getValue("delivery").jsonObject
                assertEquals("request_permission", waitingPermission.getValue("payload").jsonObject.getValue("type").jsonPrimitive.content)
                assertEquals("awaiting_permission", runtime.snapshot().getValue("turn_status").jsonPrimitive.content)
                assertNull("A visible assistant/permission wait is not completion", runtime.committedTerminalSeal("core-session-distinct"))
            }
            // The global selection changes across process death. Recovery must
            // use the admitted route/parameters, not this mutable selection.
            provider.config = OpenAiProvider.Config("https://changed.invalid/v1", "different-key", "replacement")
            AndroidApplicationRuntime(context, database).use { reopened ->
                val descriptor = loadAndroidRecoveryDescriptor(conversation, "Native recovery", run, reopened::execute)
                assertEquals(initialOwner, descriptor.assistantMessageId)
                val rejected = AndroidCanonicalAgentRuntime(reopened, descriptor)
                rejected.configure(config(requireNotNull(binding)))
                val ownershipBeforeAuthorization = reopened.execute(buildJsonObject {
                    put("type", "load_run_delivery_ownership"); put("run_id", run)
                })
                val unauthorized = runCatching {
                    rejected.recoverExistingRun(awaitCancellation = { waitMs -> kotlinx.coroutines.delay(waitMs); false }) { proof ->
                        val route = androidHttpModelRoute(OpenAiProvider.Config(endpoint.endpoint, "revoked-key", model.id))
                        require(route == proof.getValue("host_invocation").jsonObject.getValue("primary")) {
                            "Current credential authorization no longer matches admission"
                        }
                    }
                }
                assertTrue("Revoked credentials must fail before permission claim", unauthorized.isFailure)
                assertEquals("Rejected authorization must not reconcile or replace the existing lease",
                    ownershipBeforeAuthorization, reopened.execute(buildJsonObject {
                        put("type", "load_run_delivery_ownership"); put("run_id", run)
                    }))
                val runtime = AndroidCanonicalAgentRuntime(reopened, descriptor)
                runtime.configure(config(requireNotNull(binding)))
                var preparations = 0
                val recoveryOutcome = withTimeout(30_000) {
                    runtime.recoverExistingRun(awaitCancellation = { waitMs -> kotlinx.coroutines.delay(waitMs); false }) { proof ->
                        preparations++
                        assertEquals("core-session-distinct", proof.getValue("core_session_id").jsonPrimitive.content)
                        assertEquals(initialOwner, proof.getValue("assistant_message_id").jsonPrimitive.content)
                        val frozen = proof.getValue("host_invocation").jsonObject
                        val rebuilt = cloudTurnModelSnapshot(model,
                            frozenAndroidParameters(frozen, proof.getValue("context_budget").jsonObject), endpoint, provider)
                        assertEquals(frozen.getValue("primary"), rebuilt.route)
                        assertEquals(frozen.getValue("generation"), rebuilt.invocationGeneration(true))
                        assertNotEquals(frozen.getValue("primary"), androidHttpModelRoute(requireNotNull(provider.config)))
                        assertNotEquals(frozen.getValue("primary"), androidHttpModelRoute(OpenAiProvider.Config(
                            endpoint.endpoint, "revoked-key", model.id)))
                        assertEquals(binding, frozen)
                    }
                }
                val recovered = (recoveryOutcome as? AndroidRunRecoveryOutcome.Active)?.projection
                    ?: error("Permission wait recovery must remain active before its exact response")
                assertTrue(preparations >= 3)
                assertEquals("core-session-distinct", recovered.getValue("control").jsonObject.getValue("session_id").jsonPrimitive.content)
                assertNotNull(decodeAgentCoreTaskProjection(recovered["task_projection"], "core-session-distinct"))
                val driver = Driver(reopened, runtime, run)
                val permission = driver.request(recovered, "request_permission")
                val recoveredDelivery = permission.getValue("delivery").jsonObject
                for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token")) {
                    assertNotEquals("Expired recovery must issue a new fenced $key",
                        requireNotNull(originalDelivery)[key], recoveredDelivery[key])
                }
                driver.respond(permission, json("""{"type":"execution_started"}"""))
                var transition = driver.respond(permission, buildJsonObject {
                    put("type", "permission_resolved")
                    put("request_id", permission.getValue("payload").jsonObject.getValue("permission_request_id"))
                    put("resolution", corePermissionResolution(AndroidAgentPermissionDecision(
                        permission.getValue("payload").jsonObject.getValue("permission_request_id").jsonPrimitive.content,
                        AndroidAgentPermissionUserDecision.AllowOnce)))
                })
                val tool = driver.request(transition, "execute_tool")
                driver.respond(tool, json("""{"type":"execution_started"}"""))
                transition = driver.respond(tool, json("""{"type":"tool_completed","call_id":"call-read","output":{"text":"fixture read"},"outcome":"succeeded","full_output_ref":null,"artifacts":[]}"""))
                val nextModel = driver.request(transition, "model_stream")
                driver.respond(nextModel, json("""{"type":"execution_started"}"""))
                driver.respond(nextModel, json("""{"type":"model_attempt_started","provider":"fixture","model":"original-model","reason":"initial"}"""))
                val streamed = driver.respond(nextModel, buildJsonObject {
                    put("type", "model_committed_delta")
                    put("part_id", nextModel.getValue("payload").jsonObject.getValue("text_part_id"))
                    put("delta", "Recovered same Run")
                })
                assertTrue(streamed.getValue("assistant_journal_publications").jsonArray.isNotEmpty())
                transition = driver.respond(nextModel, buildJsonObject {
                    put("type", "model_completed"); put("items", buildJsonArray { add(buildJsonObject {
                        put("type", "message"); put("id", nextModel.getValue("payload").jsonObject.getValue("text_part_id"))
                        put("role", "assistant"); put("content", "Recovered same Run")
                    }) }); put("tool_calls", JsonArray(emptyList()))
                    put("usage", json("""{"input_tokens":8,"output_tokens":3,"total_tokens":11}"""))
                })
                assertEquals("completed", transition.getValue("control").jsonObject.getValue("turn_status").jsonPrimitive.content)
                assertTrue(transition.getValue("assistant_journal_publications").jsonArray.isNotEmpty())
                val seal = reopened.execute(buildJsonObject {
                    put("type", "local_assistant_run_terminal_seal"); put("conversation_id", conversation); put("run_id", run)
                })
                assertTrue("Native Application must seal its terminal transition", seal is JsonObject)
                val exactTerminal = requireNotNull(runtime.committedTerminalSeal("core-session-distinct"))
                assertEquals(run, exactTerminal.runId)
                assertEquals(initialOwner, exactTerminal.assistantMessageId)
                assertEquals("answered", exactTerminal.result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content)
                assertTrue("The committed source seal must reject a foreign Core owner",
                    runCatching { runtime.committedTerminalSeal("foreign-core") }.isFailure)
                assertEquals(JsonNull, reopened.execute(buildJsonObject { put("type", "active_run"); put("conversation_id", conversation) }))
            }
        } finally { http.close(); root.deleteRecursively() }
    }

    private class Driver(val store: AndroidApplicationRuntime, val runtime: AndroidCanonicalAgentRuntime, val run: String) {
        private val sequences = mutableMapOf<String, Long>()
        fun request(transition: JsonObject, type: String): JsonObject = transition.getValue("host_requests").jsonArray
            .map { it.jsonObject }.first { it.getValue("payload").jsonObject.getValue("type").jsonPrimitive.content == type }
        suspend fun respond(original: JsonObject, payload: JsonObject): JsonObject {
            val id = original.getValue("effect_id").jsonPrimitive.content
            val current = store.execute(buildJsonObject { put("type", "load_committed_host_requests"); put("run_id", run) })
                .jsonObject.getValue("requests").jsonArray.map { it.jsonObject }.first { it["effect_id"] == original["effect_id"] }
            // A fresh native effect has an explicit JSON null watermark. Only
            // null/missing means zero; malformed non-null values must still fail.
            val previousSequence = sequences[id]
                ?: original["resume_after_sequence"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.long
                ?: 0L
            require(previousSequence >= 0 && previousSequence < Long.MAX_VALUE)
            val sequence = previousSequence + 1
            sequences[id] = sequence
            val response = buildJsonObject {
                for (key in listOf("request_id", "effect_id", "host_id", "session_id", "turn_id", "catalog_revision", "availability_epoch", "attempt_id", "delegated_invocation_id", "lease_id"))
                    put(key, current[key] ?: JsonNull)
                put("delivery", original.getValue("delivery")); put("sequence", sequence); put("payload", payload)
            }
            return runtime.submit(buildJsonObject { put("type", "host_response"); put("response", response) }).getValue("transition").jsonObject
        }
    }

    private fun config(binding: JsonObject): JsonObject = buildJsonObject {
        put("session_id", "core-session-distinct"); put("model", "original-model")
        put("context_window", json("""{"context_window_tokens":8192,"reserved_output_tokens":1024}"""))
        put("turn_identity", buildJsonObject { put("host_invocation", binding) })
        put("tools", JsonArray(listOf(json("""{"type":"function","function":{"name":"fixture_read","parameters":{"type":"object","properties":{}}}}"""))))
        put("permission_rules", buildJsonObject { put("rules", JsonArray(listOf(permissionRule("fixture_read", decision = "ask", source = "host")))) })
        put("host_catalog", buildJsonObject {
            put("host_id", "android-device"); put("revision", 1); put("availability_epoch", 1)
            put("capabilities", buildJsonObject {
                for ((id, route) in listOf("model.stream" to "model", "permission.ask" to "permission", "fixture_read" to "tool")) {
                    put(id, buildJsonObject {
                        put("id", id); put("title", id); put("route", route); put("availability", "available")
                        put("exposure", if (route == "tool") "visible" else "internal")
                        put("input_schema", json("""{"type":"object"}"""))
                        put("output_policy", json("""{"preview_char_limit":4000,"persist_full_output":true,"allow_artifacts":true}"""))
                        put("supports_streaming", route == "model"); put("parallelism", "serial")
                        put("cancellation", "cooperative"); put("replay_safety", "recoverable")
                    })
                }
            })
        })
    }
    private fun json(value: String) = Json.parseToJsonElement(value).jsonObject
}
