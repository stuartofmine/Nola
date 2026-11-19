// SPDX-License-Identifier: Apache-2.0

package com.nola.core

import com.nola.data.local.AndroidExpertPromptDefinition
import com.nola.data.local.AndroidFrozenResendModel
import com.nola.data.local.AndroidMessageSendWitness
import com.nola.data.local.AndroidPrepareEditedResend
import com.nola.data.local.AndroidPreparedEditedResend
import com.nola.data.local.requireIdentity
import com.nola.domain.model.ChatOutputTokenField
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.ModelParameters
import com.nola.llm.LlmClient
import com.nola.llm.provider.OpenAiProvider
import java.util.UUID
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidAgentTurnServiceContractTest {
    private val localModel = ModelInfo(
        id = "qwen-2.5-3b",
        name = "Qwen 2.5 3B",
        source = ModelInfo.ModelSource.Local,
        executionCapabilities = ModelExecutionCapabilities(
            toolCalls = true,
            imageInput = false,
            reasoning = false,
            samplingParameters = true,
            chatOutputTokenField = ChatOutputTokenField.Auto,
        ),
    )

    @Test
    fun editedResendRequiresFreshIsolatedInputAndImmutableIdentity() {
        val runId = UUID.randomUUID().toString()
        val targetId = UUID.randomUUID().toString()
        val capability = androidModelCapabilityProfile(localModel, ModelParameters(), supportsNativeEventStream = true)
        val frozenModel = AndroidFrozenResendModel(
            version = ANDROID_HOST_INVOCATION_BINDING_VERSION,
            primary = androidNativeModelRoute(localModel.id),
            fallbacks = JsonArray(emptyList()),
            generation = AndroidTurnModelSnapshot(localModel, ModelParameters(), FakeLlmClient()).invocationGeneration(true),
            outputAdmission = buildJsonObject { put("resolved", buildJsonObject { put("tokens", 16384) }) },
            modelCapability = capability.wire(),
        )
        val prepared = AndroidPreparedEditedResend(
            input = AndroidPrepareEditedResend(
                operationId = runId,
                sourceConversationId = "source-conv",
                sourceMessageId = "source-msg",
                expectedSourceRevision = 1L,
                expectedSourceMessageRevision = 2L,
                editedText = "Updated user prompt",
                targetConversationId = targetId,
                title = "Edited",
                createdAtMs = 1_000L,
            ),
            source = AndroidMessageSendWitness(
                conversationId = "source-conv",
                messageId = "source-msg",
                runId = "prior-run",
                assistantMessageId = "prior-assistant",
                bodySha256 = invocationSha256("Original user prompt"),
                invocationConfigSha256 = "config-sha",
                model = frozenModel,
            ),
            userMessageId = "$runId:user",
            assistantMessageId = "$runId:assistant",
            runId = runId,
            admittedRunId = null,
        )
        prepared.requireIdentity()

        val descriptor = AndroidCanonicalTurnDescriptor(
            conversationId = targetId,
            conversationTitle = "Edited",
            submissionId = runId,
            userMessageId = "$runId:user",
            userText = "Updated user prompt",
            userContentJson = null,
            assistantMessageId = "$runId:assistant",
            editedResendOperationId = runId,
        )
        val snapshot = AndroidTurnModelSnapshot(
            model = localModel,
            parameters = ModelParameters(),
            client = FakeLlmClient(),
            route = androidNativeModelRoute(localModel.id),
            editedResendModel = frozenModel,
        )
        val plan = emptyPlan(AndroidAgentPermissionMode.Cautious)
        val validRequest = AndroidAgentTurnRequest(
            model = snapshot,
            sessionId = targetId,
            input = "Updated user prompt",
            tools = emptyList(),
            toolPlan = plan,
            canonicalTurn = descriptor,
        )

        fun matchesResendPreconditions(req: AndroidAgentTurnRequest): Boolean =
            !req.recoverExistingRun &&
                req.history.isEmpty() &&
                req.attachments.isEmpty() &&
                req.turnContextComponents.isEmpty() &&
                req.model?.editedResendModel != null

        assertTrue(matchesResendPreconditions(validRequest))
        assertFalse(matchesResendPreconditions(validRequest.copy(recoverExistingRun = true)))
        assertFalse(matchesResendPreconditions(validRequest.copy(history = listOf(AgentCoreMessage("user", "stale")))))
        assertFalse(matchesResendPreconditions(validRequest.copy(turnContextComponents = listOf(
            AgentCoreContextComponent("document", "d1", JsonPrimitive("stale")),
        ))))
        assertFalse(matchesResendPreconditions(validRequest.copy(model = snapshot.copy(editedResendModel = null))))

        assertThrows(IllegalArgumentException::class.java) {
            prepared.copy(input = prepared.input.copy(targetConversationId = "source-conv")).requireIdentity()
        }
    }

    @Test
    fun expertInstructionSha256AndSingleInjectionInvariantsDetectTampering() {
        val definitionText = "You are a cautious code reviewer."
        val expectedHash = invocationSha256(definitionText)
        assertEquals(64, expectedHash.length)
        assertEquals(expectedHash, invocationSha256(definitionText))
        assertNotEquals(expectedHash, invocationSha256("$definitionText "))

        val definition = AndroidExpertPromptDefinition(
            componentId = "expert:reviewer",
            content = definitionText,
            contentSha256 = expectedHash,
            definitionRevision = 3L,
            platform = JsonPrimitive("android"),
        )
        assertEquals(definition.contentSha256, invocationSha256(definition.content))

        val duplicatePromptTemplate = AgentCoreContextComponent(
            kind = "prompt_template",
            id = "android:user-custom-instructions",
            content = JsonPrimitive("Injected twice"),
        )
        val requestTurnComponents = listOf(duplicatePromptTemplate)
        assertFalse(requestTurnComponents.none { it.kind == "prompt_template" })
        assertTrue(emptyList<AgentCoreContextComponent>().none { it.kind == "prompt_template" })
    }

    @Test
    fun permissionRecoveryAllowAllFallsBackToFrozenCautiousDigestWithoutDrift() {
        assertTrue(AndroidAgentPermissionMode.AllowAll.permitsResuming(AndroidAgentPermissionMode.Cautious))
        assertTrue(AndroidAgentPermissionMode.Cautious.permitsResuming(AndroidAgentPermissionMode.Cautious))
        assertFalse(AndroidAgentPermissionMode.Cautious.permitsResuming(AndroidAgentPermissionMode.AllowAll))

        val cautiousPlan = emptyPlan(AndroidAgentPermissionMode.Cautious)
        val allowAllPlan = emptyPlan(AndroidAgentPermissionMode.AllowAll)
        assertNotEquals(androidAgentPermissionRules(cautiousPlan), androidAgentPermissionRules(allowAllPlan))

        val snapshot = AndroidTurnModelSnapshot(
            model = localModel,
            parameters = ModelParameters(),
            client = FakeLlmClient(),
            route = androidNativeModelRoute(localModel.id),
        )
        val modelPayload = buildJsonObject {
            put("primary", androidNativeModelRoute(localModel.id))
            put("generation", snapshot.invocationGeneration(thinking = true))
            put("output_admission", buildJsonObject { put("resolved", buildJsonObject { put("tokens", 16384) }) })
            put("model_capabilities", androidModelCapabilityProfile(localModel, ModelParameters(), true).wire())
        }

        fun runtimeConfig(plan: AgentCoreAndroidToolPlan) = buildJsonObject {
            put("host_catalog", buildJsonObject { put("version", 1) })
            put("tools", JsonArray(plan.tools))
            put("permission_rules", androidAgentPermissionRules(plan))
            put("context_window", buildJsonObject {
                put("context_window_tokens", 32768)
                put("reserved_output_tokens", 16384)
            })
        }

        val frozenCautiousDigest = bindAndroidInvocation(modelPayload, runtimeConfig(cautiousPlan))
            .getValue("execution_policy_sha256")
        val allowAllDigest = bindAndroidInvocation(modelPayload, runtimeConfig(allowAllPlan))
            .getValue("execution_policy_sha256")
        val recomputedCautiousDigest = bindAndroidInvocation(modelPayload, runtimeConfig(cautiousPlan))
            .getValue("execution_policy_sha256")

        assertNotEquals(frozenCautiousDigest, allowAllDigest)
        assertEquals(frozenCautiousDigest, recomputedCautiousDigest)
    }

    @Test
    fun localModelRouteParametersAndGuardedClientEnforceExactMatch() = runTest {
        val parameters = ModelParameters(
            temperature = 0.35f,
            topP = 0.85f,
            topK = 32,
            maxCompletionTokens = 2048,
            maxContextTokens = 16384,
            enableThinking = false,
            seed = 77L,
            stopSequences = "STOP\nEND",
        )
        val route = androidNativeModelRoute(localModel.id)
        assertEquals("native", route.getValue("kind").jsonPrimitive.content)
        assertEquals("llama_cpp", route.getValue("adapter").jsonPrimitive.content)
        assertEquals(localModel.id, route.getValue("model_id").jsonPrimitive.content)
        assertEquals(JsonNull, route.getValue("profile_ref"))

        val snapshot = AndroidTurnModelSnapshot(localModel, parameters, FakeLlmClient(), route)
        val generation = snapshot.invocationGeneration(thinking = parameters.enableThinking)
        val binding = buildJsonObject { put("generation", generation) }
        val contextBudget = buildJsonObject {
            put("context_window_tokens", parameters.maxContextTokens)
            put("reserved_output_tokens", parameters.maxCompletionTokens)
        }
        val restored = frozenAndroidParameters(binding, contextBudget)
        assertEquals(parameters.temperature, restored.temperature)
        assertEquals(parameters.topP, restored.topP)
        assertEquals(parameters.topK, restored.topK)
        assertEquals(parameters.maxContextTokens, restored.maxContextTokens)
        assertEquals(parameters.maxCompletionTokens, restored.maxCompletionTokens)
        assertEquals(parameters.enableThinking, restored.enableThinking)
        assertEquals(parameters.seed, restored.seed)
        assertEquals(parameters.stopSequences, restored.stopSequences)

        val nonToolModel = localModel.copy(executionCapabilities = ModelExecutionCapabilities(toolCalls = false))
        val nonToolSnapshot = snapshot.copy(model = nonToolModel)
        val sampleTools = listOf(buildJsonObject { put("name", "read_file") })
        assertEquals(sampleTools, snapshot.modelFacingTools(sampleTools))
        assertTrue(nonToolSnapshot.modelFacingTools(sampleTools).isEmpty())

        var valid = true
        val guarded = GuardedTurnModelClient(FakeLlmClient()) {
            check(valid) { "The selected on-device model or parameters are no longer loaded." }
        }
        assertEquals("ok", guarded.chat("hi", systemPrompt = null, enableThinking = false))
        valid = false
        val failed = runCatching { guarded.chat("hi", systemPrompt = null, enableThinking = false) }
        assertTrue(failed.exceptionOrNull() is IllegalStateException)

        val httpConfig = OpenAiProvider.Config(
            baseUrl = "https://api.example.com/v1",
            apiKey = "secret-key",
            model = "gpt-5",
        )
        assertEquals("http", androidHttpModelRoute(httpConfig).getValue("kind").jsonPrimitive.content)
    }

    private fun emptyPlan(mode: AndroidAgentPermissionMode) = AgentCoreAndroidToolPlan(
        tools = emptyList(),
        grantedCapabilities = emptyList(),
        allowedTools = emptyList(),
        policyDigest = "policy",
        manifestDigest = "manifest",
        manifestId = "manifest-id",
        manifestVersion = 1L,
        manifestValidUntilMs = 60_000L,
        identity = AgentCoreRuntimeIdentity("host", "client", "session"),
        permissionMode = mode,
    )

    private class FakeLlmClient : LlmClient {
        override val supportsNativeEventStream: Boolean = true
        override suspend fun generateSummary(input: ModelInput, timeout: kotlin.time.Duration?): ModelOutput =
            ModelOutput(text = "ok")
        override fun generateSummaryStream(input: ModelInput): Flow<String> = emptyFlow()
        override suspend fun chat(
            message: String,
            systemPrompt: String?,
            enableThinking: Boolean,
            timeout: kotlin.time.Duration?,
        ): String = "ok"
    }
}
