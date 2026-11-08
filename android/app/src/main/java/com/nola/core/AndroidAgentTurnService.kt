@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package com.nola.core

import android.content.Context
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.repository.ModelRepository
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.expertChildInstructions
import com.nola.llm.ModelService
import com.nola.domain.model.ModelAttachment
import com.nola.work.MemoryWorkerScheduler
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.selects.onTimeout
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import kotlinx.serialization.json.*

data class AndroidAgentTurnRequest(
    val model: AndroidTurnModelSnapshot?,
    val sessionId: String,
    val input: String,
    val history: List<AgentCoreMessage> = emptyList(),
    val tools: List<JsonElement>,
    val toolPlan: AgentCoreAndroidToolPlan,
    val config: AgentCoreRuntimeConfig? = null,
    val inputOrigin: DeviceAgentInputOrigin = DeviceAgentInputOrigin.INTERACTIVE_USER,
    val turnContextComponents: List<AgentCoreContextComponent> = emptyList(),
    val responseStyle: String? = null,
    val customSystemPrompt: String? = null,
    val enableThinking: Boolean? = null,
    val attachments: List<ModelAttachment> = emptyList(),
    val canonicalTurn: AndroidCanonicalTurnDescriptor,
    val recoverExistingRun: Boolean = false,
    val resultEgressAuthorized: Boolean = false,
    val delegationEnabled: Boolean = false,
)

class AndroidAgentTurnHandle internal constructor(
    internal val coordinator: DeviceAgentCoordinator,
    internal val coreSessionId: String,
    val events: Flow<DeviceAgentCoordinatorEvent>,
) {
    suspend fun resolvePermission(decision: AndroidAgentPermissionDecision) =
        coordinator.resolvePermission(decision)

    fun requestCancellation(reason: String = "user_requested"): Boolean =
        coordinator.requestCancellation(reason)
}

@Singleton
class AndroidAgentTurnService @Inject constructor(
    private val modelService: ModelService,
    private val modelRepository: ModelRepository,
    private val modelParametersStore: ModelParametersStore,
    private val openAiProvider: com.nola.llm.provider.OpenAiProvider,
    private val localLlamaProvider: com.nola.llm.provider.LocalLlamaProvider,
    private val deviceModelStore: com.nola.llm.local.DeviceModelStore,
    private val memoryProfileDataStore: MemoryProfileDataStore,
    private val memoryWorkerScheduler: MemoryWorkerScheduler,
    private val applicationRuntime: AndroidApplicationRuntime,
    @ApplicationContext context: Context,
) {
    internal val permissionBroker = AndroidAgentPermissionBroker(context)
    internal val hostExecutor = AndroidAgentHostExecutor(context, memoryProfileDataStore)
    private val memoryWorker = AndroidCanonicalMemoryWorkerService(applicationRuntime)

    private val childScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val childWake = Channel<Unit>(Channel.CONFLATED)

    init {
        AndroidEmbeddedChildExecutor.onTaskChanged = { resumeEmbeddedChildren() }
        childScope.launch {
            for (ignored in childWake) {
                try {
                    var continueWaiting = true
                    while (continueWaiting) {
                        val plan = applicationRuntime.executeQuery(buildJsonObject {
                            put("type", "load_mobile_task_wake_plan")
                            put("now_ms", System.currentTimeMillis())
                        }).jsonObject
                        for (candidate in plan.getValue("candidates").jsonArray) {
                            try { resumeEmbeddedChild(candidate.jsonObject) }
                            catch (cancel: CancellationException) { throw cancel }
                            catch (error: Exception) {
                                android.util.Log.e("NolaChildHost", "Child recovery authorization unavailable", error)
                            }
                        }
                        val next = plan["next_wake_at_ms"]?.jsonPrimitive?.longOrNull
                        if (next == null) continueWaiting = false
                        else {
                            val wait = (next - plan.getValue("observed_at_ms").jsonPrimitive.long).coerceAtLeast(1)
                            select<Unit> { childWake.onReceive { }; onTimeout(wait) { } }
                        }
                    }
                } catch (cancel: CancellationException) { throw cancel }
                catch (error: Exception) {
                    android.util.Log.e("NolaChildHost", "Core child wake observation failed", error)
                }
            }
        }
    }

    /** Startup/foreground and committed task changes only wake the Core plan reader. */
    fun resumeEmbeddedChildren() { childWake.trySend(Unit) }

    private suspend fun resumeEmbeddedChild(candidate: JsonObject) {
        val taskId = candidate.getValue("task_id").jsonPrimitive.content
        if (AndroidEmbeddedChildExecutor.isActive(taskId)) return
        val proof = candidate.getValue("parent_invocation").jsonObject
        val parentId = candidate.getValue("parent_conversation_id").jsonPrimitive.content
        require(proof.getValue("conversation_id").jsonPrimitive.content == parentId)
        val model = recoverModel(proof)
        val descriptor = AndroidCanonicalTurnDescriptor(parentId, "",
            proof.getValue("run_id").jsonPrimitive.content, "", "", null,
            proof.getValue("assistant_message_id").jsonPrimitive.content)
        val parent = AndroidCanonicalAgentRuntime(applicationRuntime, descriptor)
        val parameters = model.parameters
        val modelClient = TextAgentModelClient(model.client, parameters.enableThinking)
        AndroidEmbeddedChildExecutor.start(taskId, parent) { config ->
            require(config.getValue("session_id").jsonPrimitive.content == candidate.getValue("child_conversation_id").jsonPrimitive.content)
            val session = config.getValue("session_id").jsonPrimitive.content
            val policy = androidChildExecutionPolicy(config)
            val refresh = { toolPlan(session, policy.resultEgressAuthorized, policy.permissionMode) }
            val livePlan = refresh()
            AndroidPreparedChildExecutor(livePlan.tools) { child ->
                val coordinator = DeviceAgentCoordinator(modelClient, permissionBroker, hostExecutor, child,
                    androidModelCapabilityProfile(model.model, parameters, model.client.supportsNativeEventStream),
                    refreshToolPlan = refresh, embeddedMailbox = true)
                val runtime = AgentCoreRuntimeConfig(model = model.model.id,
                    contextWindowTokens = parameters.maxContextTokens,
                    reservedOutputTokens = parameters.maxCompletionTokens)
                val execute: suspend (JsonObject, AgentCoreTransitionRuntime, DeviceAgentCoordinator.RunState) -> Unit = { envelope, sink, state ->
                    coordinator.executeEmbeddedRequest(envelope, sink, runtime, livePlan, state, parentId)
                }
                execute
            }
        }
    }

    fun toolPlan(
        sessionId: String,
        resultEgressAuthorized: Boolean,
        permissionMode: AndroidAgentPermissionMode = AndroidAgentPermissionMode.Cautious,
    ): AgentCoreAndroidToolPlan = AgentCoreToolCatalog.plan(
        executionMode = AndroidAgentExecutionMode.DeviceLocal,
        permissionMode = permissionMode,
        permissionBroker = permissionBroker,
        executor = hostExecutor,
        identity = permissionBroker.runtimeIdentity(sessionId),
        resultEgressAuthorized = resultEgressAuthorized,
        uiToolSchema = applicationRuntime.execute(buildJsonObject { put("type", "read_ui_capability") })
            .jsonObject.getValue("model_tool"),
    )

    suspend fun start(originalRequest: AndroidAgentTurnRequest): AndroidAgentTurnHandle {
        val application = AndroidCanonicalAgentRuntime(applicationRuntime, originalRequest.canonicalTurn)
        val frozen = if (originalRequest.recoverExistingRun) application.loadExactRunInvocation() else null
        val request = if (frozen == null) originalRequest else originalRequest.copy(
            model = recoverModel(frozen),
            sessionId = frozen.getValue("core_session_id").jsonPrimitive.content,
            toolPlan = toolPlan(frozen.getValue("core_session_id").jsonPrimitive.content,
                originalRequest.resultEgressAuthorized, originalRequest.toolPlan.permissionMode),
            enableThinking = frozen.getValue("host_invocation").jsonObject.getValue("generation")
                .jsonObject.getValue("thinking_enabled").jsonPrimitive.boolean,
        )
        val model = checkNotNull(request.model) { "A new Run requires a captured model" }
        val parameters = model.parameters
        val thinking = request.enableThinking ?: parameters.enableThinking
        val prepared = prepareInvocation(model, thinking)
        val modelFacingTools = model.modelFacingTools(if (frozen == null) request.tools else request.toolPlan.tools)
        val promptComposition = CoreAgentPromptComposer.compose(AndroidSystemPrompt.agentFragments(
            context = applicationContext,
            responseStyle = request.responseStyle ?: parameters.responseStyle,
            customSystemPrompt = "", // The user definition below enters once as a typed component.
        ))
        // Session history is conversational data. A restored, imported, or
        // stale system item must never replace the current Core-owned policy.
        val isEditedResend = request.canonicalTurn.editedResendOperationId != null
        require(!isEditedResend || (!request.recoverExistingRun && request.history.isEmpty() &&
            request.attachments.isEmpty() && request.turnContextComponents.isEmpty() && model.editedResendModel != null)) {
            "Edited resend requires a fresh isolated input"
        }
        // Definitions come from this Store's authorized complete manifest, for
        // both ordinary expert sends and edited children. The current model's
        // custom prompt must not be appended a second time or replace them.
        val expertDefinitions = applicationRuntime.expertChildInstructions(
            request.canonicalTurn.conversationId, request.canonicalTurn.submissionId)
        val definitionComponents = expertDefinitions.orEmpty().map {
            require(invocationSha256(it.content) == it.contentSha256) { "Original instruction definition digest changed" }
            AgentCoreContextComponent(kind = "prompt_template", id = it.componentId, content = JsonPrimitive(it.content),
                source = AgentCoreContextComponentSource(platform = "android", title = "Expert instructions"))
        }
        require(expertDefinitions == null || request.turnContextComponents.none { it.kind == "prompt_template" }) {
            "Expert instructions must enter once through the authorized source manifest"
        }
        val turnComponents = if (isEditedResend) listOf(AgentCoreContextComponent(
            kind = "system", id = "android:fresh-host-system-policy", content = JsonPrimitive(promptComposition.rendered),
            source = AgentCoreContextComponentSource(platform = "android", title = "Current Host policy"))) + definitionComponents
        else request.turnContextComponents + if (expertDefinitions != null) definitionComponents
            else listOfNotNull(AndroidSystemPrompt.customInstructionComponent(request.customSystemPrompt ?: parameters.customSystemPrompt))
        val effectiveHistory = if (isEditedResend) emptyList() else
            listOf(AgentCoreMessage(role = "system", content = promptComposition.rendered)) +
                request.history.filter { it.role != "system" }
        val coordinator = DeviceAgentCoordinator(
            modelClient = TextAgentModelClient(
                client = model.client,
                enableThinking = thinking,
                attachments = request.attachments,
            ),
            permissionBroker = permissionBroker,
            hostExecutor = hostExecutor,
            canonicalApplication = application,
            modelCapability = model.editedResendModel?.let { frozenAndroidModelCapability(it.modelCapability) }
                ?: androidModelCapabilityProfile(model.model, parameters, model.client.supportsNativeEventStream),
            childToolPlan = { childSession -> toolPlan(childSession, request.resultEgressAuthorized, request.toolPlan.permissionMode) },
            captureDelegation = request.delegationEnabled && !request.recoverExistingRun,
            refreshToolPlan = { toolPlan(request.sessionId, request.resultEgressAuthorized, request.toolPlan.permissionMode) },
            prepareRecovery = { proof, config ->
                // Core ResumeHostEffect advances only the replay witness. The
                // admitted invocation and every authority field remain exact.
                val replayWitness = setOf("checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256")
                require(proof.filterKeys { it !in replayWitness } == checkNotNull(frozen).filterKeys { it !in replayWitness }) {
                    "The prepared recovery Run changed"
                }
                // Resolve current local authority again after the delivery wait.
                val authorized = recoverModel(proof)
                val livePlan = toolPlan(request.sessionId, request.resultEgressAuthorized, request.toolPlan.permissionMode)
                revalidateFrozenAndroidToolPlan(request.toolPlan, livePlan)
                val preparedRecovery = prepareInvocation(authorized, thinking)
                val receipt = applicationRuntime.execute(buildJsonObject {
                    put("type", "validate_mobile_run_recovery_authority")
                    put("conversation_id", proof.getValue("conversation_id"))
                    put("expected_run_id", proof.getValue("run_id"))
                    put("assistant_message_id", proof.getValue("assistant_message_id"))
                    put("expected_invocation_config_digest", proof.getValue("invocation_config_digest"))
                    put("current", buildJsonObject {
                        for (field in listOf("host_catalog", "permission_rules", "context_window"))
                            put(field, config.getValue(field))
                        put("tools", JsonArray(authorized.modelFacingTools(config.getValue("tools").jsonArray)))
                        put("host_invocation", bindAndroidInvocation(preparedRecovery, config))
                    })
                }).jsonObject
                for (field in listOf("conversation_id", "run_id", "assistant_message_id", "invocation_config_digest"))
                    require(receipt[field] == proof[field]) { "Recovery authority receipt owner changed" }
                livePlan
            },
        )
        val runtimeConfig = (request.config ?: AgentCoreRuntimeConfig(
            executionMode = AndroidAgentExecutionMode.DeviceLocal.wireValue,
            maxIterations = 0,
            contextWindowTokens = parameters.maxContextTokens,
            grantedCapabilities = request.toolPlan.grantedCapabilities,
        )).copy(
            model = model.model.id,
            preparedHostInvocation = prepared,
            contextWindowTokens = parameters.maxContextTokens,
            reservedOutputTokens = parameters.maxCompletionTokens,
            stablePromptDigest = promptComposition.stablePrefixSha256,
            promptFragmentIds = promptComposition.fragmentIds,
            stablePromptFragmentDigests = promptComposition.stableFragmentSha256,
        )
        return AndroidAgentTurnHandle(
            coordinator = coordinator,
            coreSessionId = request.sessionId,
            events = flow {
                emitAll(
                    coordinator.run(
                        sessionId = request.sessionId,
                        input = request.input,
                        history = effectiveHistory,
                        tools = modelFacingTools,
                        toolPlan = request.toolPlan,
                        config = runtimeConfig,
                        inputOrigin = request.inputOrigin,
                        turnContextComponents = turnComponents,
                        recoverExistingRun = request.recoverExistingRun,
                    ),
                )
                memoryWorkerScheduler.enqueue(request.canonicalTurn.conversationId)
            },
        )
    }

    /** Recovery does not require an active/current model selection. */
    fun recoveryDescriptor(conversationId: String, conversationTitle: String, runId: String): AndroidCanonicalTurnDescriptor =
        loadAndroidRecoveryDescriptor(conversationId, conversationTitle, runId, applicationRuntime::executeQuery)

    fun inspectUnknownRecovery(descriptor: AndroidCanonicalTurnDescriptor): com.nola.data.local.UnknownEffectRecoveryInspection? {
        val observed = applicationRuntime.executeQuery(buildJsonObject {
            put("type", "inspect_run_recovery")
            put("conversation_id", descriptor.conversationId)
            put("expected_run_id", descriptor.submissionId)
            put("assistant_message_id", descriptor.assistantMessageId)
        }).jsonObject
        require(observed["conversation_id"]?.jsonPrimitive?.content == descriptor.conversationId &&
            observed["run_id"]?.jsonPrimitive?.content == descriptor.submissionId &&
            observed["assistant_message_id"]?.jsonPrimitive?.content == descriptor.assistantMessageId)
        val facts = observed.getValue("observation").jsonObject
        require(facts["state"]?.jsonPrimitive?.content == "active" && facts["source_host_id"] == JsonNull) {
            "Recovery decisions belong to the execution Host"
        }
        return applicationRuntime.inspectUnknownEffectRecovery(descriptor.conversationId,
            descriptor.submissionId, descriptor.assistantMessageId)
    }

    fun pendingUnknownRecovery(conversationId: String) = applicationRuntime.pendingUnknownEffectDecision(conversationId)
    fun submitUnknownRecovery(decision: com.nola.data.local.UnknownEffectRecoveryDecision, requestId: String) =
        applicationRuntime.submitUnknownEffectDecision(decision, requestId)
    fun recoverUnknownRecovery(expected: com.nola.data.local.PendingUnknownEffectRecovery) =
        applicationRuntime.recoverPendingUnknownEffectDecision(expected)

    fun unknownRecoveryTerminal(conversationId: String, runId: String, expectedAssistant: String?): AndroidCanonicalRunTerminalSeal? {
        val raw = applicationRuntime.executeQuery(buildJsonObject {
            put("type", "local_assistant_run_terminal_seal")
            put("conversation_id", conversationId)
            put("run_id", runId)
        })
        if (raw == JsonNull) return null
        val seal = raw.jsonObject
        val identity = seal.getValue("identity").jsonObject
        val assistant = identity.getValue("assistant_message_id").jsonPrimitive.content
        require(expectedAssistant == null || expectedAssistant == assistant)
        val descriptor = AndroidCanonicalTurnDescriptor(conversationId, "", runId, "", "", null, assistant)
        return loadAndroidCanonicalTerminalSeal(descriptor, seal.getValue("core_session_id").jsonPrimitive.content) { raw }
    }

    /** Recovery does not require an active/current model selection. */
    suspend fun captureRecoveryModel(descriptor: AndroidCanonicalTurnDescriptor): AndroidTurnModelSnapshot =
        recoverModel(AndroidCanonicalAgentRuntime(applicationRuntime, descriptor).loadExactRunInvocation())

    suspend fun captureModel(
        model: com.nola.domain.model.ModelInfo,
        parameters: com.nola.domain.model.ModelParameters,
    ): AndroidTurnModelSnapshot = when (model.source) {
        com.nola.domain.model.ModelInfo.ModelSource.Cloud -> {
            val current = model.profileId?.let { profileId ->
                modelRepository.listConfiguredModels().firstOrNull {
                    it.profileId == profileId && it.id == model.id
                } ?: error("The selected cloud model profile is no longer available")
            } ?: model
            cloudTurnModelSnapshot(
                current, parameters,
                checkNotNull(modelRepository.getApiModelConfigFor(current, parameters)) {
                    "The selected cloud model is not available on the configured endpoint"
                },
                openAiProvider,
            )
        }
        com.nola.domain.model.ModelInfo.ModelSource.Local -> {
            val validate = {
                val loaded = localLlamaProvider.parameters
                check(localLlamaProvider.activeModel?.id == model.id &&
                    loaded.temperature == parameters.temperature && loaded.topP == parameters.topP &&
                    loaded.topK == parameters.topK && loaded.maxContextTokens == parameters.maxContextTokens &&
                    loaded.maxCompletionTokens == parameters.maxCompletionTokens) {
                    "The selected on-device model or parameters are no longer loaded. Load this model before sending."
                }
            }
            validate()
            AndroidTurnModelSnapshot(model, parameters, localLlamaProvider.clientForTurn(model.id, localLlamaProvider.parameters), androidNativeModelRoute(model.id))
        }
        com.nola.domain.model.ModelInfo.ModelSource.WebHost -> error("Web-host execution requires its own host turn route")
    }

    /** Rebuild only the exact original model. Current registry supplies authorization, never old policy. */
    suspend fun captureEditedResendModel(frozen: com.nola.data.local.AndroidFrozenResendModel): AndroidTurnModelSnapshot {
        require(frozen.version == ANDROID_HOST_INVOCATION_BINDING_VERSION && frozen.fallbacks.isEmpty()) {
            "The original model route is unsupported by this Android Host"
        }
        val capability = frozenAndroidModelCapability(frozen.modelCapability)
        val parameters = frozenAndroidParameters(buildJsonObject { put("generation", frozen.generation) }, buildJsonObject {
            put("context_window_tokens", capability.contextWindowTokens)
            put("reserved_output_tokens", frozen.generation.getValue("max_output_tokens"))
        })
        require(parameters.maxCompletionTokens <= capability.maxOutputTokens)
        val candidates = when (frozen.primary.getValue("kind").jsonPrimitive.content) {
            "http" -> modelRepository.listConfiguredModels()
            "native" -> {
                require(frozen.primary.getValue("adapter").jsonPrimitive.content == "llama_cpp")
                val id = frozen.primary.getValue("model_id").jsonPrimitive.content
                require(deviceModelStore.installedModels().any { it.id == id }) { "The original on-device model is not installed" }
                // Starting a different model is a separate explicit model-center action.
                listOf(com.nola.domain.model.ModelInfo(id, id, com.nola.domain.model.ModelInfo.ModelSource.Local))
            }
            else -> error("The original model route is unsupported by this Android Host")
        }
        val matches = mutableListOf<AndroidTurnModelSnapshot>()
        for (candidate in candidates) {
            if (capability.id != "android-text-adapter-v1:${candidate.source.name}:${candidate.selectionId}" ||
                capability.revision != (candidate.profileRevision ?: 1)) continue
            val outputField = when (frozen.primary["output_token_field"]?.jsonPrimitive?.content) {
                "max_tokens" -> com.nola.domain.model.ChatOutputTokenField.MaxTokens
                "max_completion_tokens", "max_output_tokens", null -> com.nola.domain.model.ChatOutputTokenField.MaxCompletionTokens
                else -> error("Unsupported original output token field")
            }
            val original = candidate.copy(isVisionModel = capability.visionInput,
                executionCapabilities = com.nola.domain.model.ModelExecutionCapabilities(
                    toolCalls = capability.nativeToolCalls, imageInput = capability.visionInput,
                    reasoning = capability.reasoning,
                    samplingParameters = frozen.generation.getValue("sampling_override").jsonPrimitive.boolean,
                    chatOutputTokenField = outputField))
            val captured = try {
                if (original.source == com.nola.domain.model.ModelInfo.ModelSource.Cloud) {
                    val endpoint = modelRepository.getApiModelConfigFor(candidate) ?: continue
                    cloudTurnModelSnapshot(original, parameters, endpoint, openAiProvider)
                } else captureModel(original, parameters)
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { continue }
            if (capability.nativeEventStream != captured.client.supportsNativeEventStream) continue
            val actual = prepareInvocation(captured, parameters.enableThinking)
            if (actual["primary"] == frozen.primary && actual["generation"] == frozen.generation &&
                actual["output_admission"] == frozen.outputAdmission) matches += captured.copy(editedResendModel = frozen)
        }
        require(matches.size == 1) { if (matches.isEmpty()) "The original model or its current credential authorization has changed"
            else "The original model matches multiple authorized profiles; select an unambiguous configuration" }
        val selected = matches.single()
        val guarded = GuardedTurnModelClient(selected.client) {
            if (selected.model.source == com.nola.domain.model.ModelInfo.ModelSource.Cloud) {
                val current = modelRepository.listConfiguredModels().singleOrNull {
                    it.selectionId == selected.model.selectionId && it.id == selected.model.id && it.profileRevision == selected.model.profileRevision
                } ?: error("The original model authorization changed")
                val endpoint = checkNotNull(modelRepository.getApiModelConfigFor(current)) { "The original model credential authorization was revoked" }
                val revalidated = cloudTurnModelSnapshot(selected.model, selected.parameters, endpoint, openAiProvider)
                require(revalidated.route == frozen.primary) { "The original model credential or endpoint changed" }
            } else captureModel(selected.model, selected.parameters)
        }
        return selected.copy(client = guarded)
    }

    internal suspend fun goalModelAdmission(model: com.nola.domain.model.ModelInfo,
        parameters: com.nola.domain.model.ModelParameters, thinking: Boolean): JsonObject {
        val invocation=prepareInvocation(captureModel(model, parameters), thinking)
        return buildJsonObject { for (key in listOf("primary","generation","output_admission")) put(key,invocation.getValue(key)) }
    }

    private fun prepareInvocation(model: AndroidTurnModelSnapshot, thinking: Boolean): JsonObject {
        val route = checkNotNull(model.route) { "Model has no admitted execution route" }
        val output = applicationRuntime.execute(buildJsonObject {
            put("type", "admit_host_output")
            put("routes", buildJsonArray { add(buildJsonObject {
                put("route", route); put("max_input_tokens", JsonNull)
                put("max_output_tokens", JsonNull); put("max_total_tokens", JsonNull)
            }) })
            put("preference", buildJsonObject { put("explicit", model.parameters.maxCompletionTokens) })
        }).jsonObject
        require(output.getValue("resolved").jsonObject.getValue("tokens").jsonPrimitive.int == model.parameters.maxCompletionTokens)
        model.editedResendModel?.let { original ->
            require(output == original.outputAdmission && model.route == original.primary &&
                model.invocationGeneration(thinking) == original.generation) { "The original model invocation changed before admission" }
        }
        return buildJsonObject {
            put("primary", output.getValue("routes").jsonArray.single().jsonObject.getValue("route"))
            put("generation", model.invocationGeneration(thinking)); put("output_admission", output)
            put("model_capabilities", Json.encodeToJsonElement(com.nola.domain.model.ModelExecutionCapabilities.serializer(), model.model.executionCapabilities))
        }
    }

    private suspend fun recoverModel(frozen: JsonObject): AndroidTurnModelSnapshot {
        val binding = frozen.getValue("host_invocation").jsonObject
        require(binding.getValue("version").jsonPrimitive.int == ANDROID_HOST_INVOCATION_BINDING_VERSION && binding.getValue("fallbacks").jsonArray.isEmpty())
        val route = binding.getValue("primary").jsonObject
        val parameters = frozenAndroidParameters(binding, frozen.getValue("context_budget").jsonObject)
        val candidates = when (route.getValue("kind").jsonPrimitive.content) {
            "http" -> modelRepository.listConfiguredModels()
            "native" -> {
                require(route.getValue("adapter").jsonPrimitive.content == "llama_cpp")
                val id = route.getValue("model_id").jsonPrimitive.content
                val installed = deviceModelStore.installedModels().firstOrNull { it.id == id }
                    ?: error("The admitted on-device model is no longer installed")
                if (localLlamaProvider.activeModel == null) localLlamaProvider.start(installed, parameters)
                listOf(com.nola.domain.model.ModelInfo(id, id, com.nola.domain.model.ModelInfo.ModelSource.Local))
            }
            else -> error("Unsupported frozen Android model adapter")
        }
        for (candidate in candidates) {
            // Missing/revoked profiles and changed secrets cannot become fallback routes.
            val captured = try { captureModel(candidate, parameters) } catch (cancel: kotlinx.coroutines.CancellationException) {
                throw cancel
            } catch (failure: Exception) {
                android.util.Log.w("NolaAgentProtocol", "recovery_model_capture_unavailable type=${failure.javaClass.simpleName}")
                continue
            }
            val admission = prepareInvocation(captured, parameters.enableThinking)
            val admittedRoute = admission.getValue("primary").jsonObject
            val generation = admission.getValue("generation").jsonObject
            if (admittedRoute != route || generation != binding["generation"]) {
                val routeDifferences = (route.keys + admittedRoute.keys).filter { route[it] != admittedRoute[it] }
                val frozenGeneration = binding.getValue("generation").jsonObject
                val generationDifferences = (generation.keys + frozenGeneration.keys).filter { generation[it] != frozenGeneration[it] }
                android.util.Log.w("NolaAgentProtocol", "recovery_model_binding_mismatch route_fields=$routeDifferences generation_fields=$generationDifferences")
                continue
            }
            if (admission["output_admission"] != binding["output_admission"]) {
                android.util.Log.w("NolaAgentProtocol", "recovery_output_admission_mismatch")
                continue
            }
            return captured
        }
        error("The admitted model or current credential authorization is unavailable")
    }

    suspend fun captureActiveModel(): AndroidTurnModelSnapshot {
        ensureConfiguredModel()
        val id = modelService.currentModelId
        val source = when (modelService.currentModelSource) {
            "local" -> com.nola.domain.model.ModelInfo.ModelSource.Local
            "cloud" -> com.nola.domain.model.ModelInfo.ModelSource.Cloud
            else -> error("No device/API model is activated for background work")
        }
        val profileId = modelService.currentProfileId
        return captureModel(com.nola.domain.model.ModelInfo(id, id, source, profileId = profileId),
            modelParametersStore.loadParameters(profileId ?: id))
    }

    suspend fun runPendingMemoryWorkers(
        sessionId: String,
        maximumJobCount: Int = 2,
    ): Int {
        return runPendingMemoryWorkersReport(sessionId, maximumJobCount).completed
    }

    suspend fun runPendingMemoryWorkersReport(
        sessionId: String,
        maximumJobCount: Int = 2,
    ): AndroidMemoryWorkerRunReport {
        val model = captureActiveModel()
        val parameters = model.parameters
        val coreConfig = kotlinx.serialization.json.buildJsonObject {
            put("session_id", sessionId)
            put("model", model.model.id)
            put("memory_policy", automaticMemoryPolicyConfig())
            put(
                "context_window",
                kotlinx.serialization.json.buildJsonObject {
                    put("context_window_tokens", parameters.maxContextTokens)
                    put("reserved_output_tokens", com.nola.domain.model.ModelParameterLimits.validatedOutputTokens(parameters.maxCompletionTokens))
                },
            )
        }
        return memoryWorker.run(
            conversationId = sessionId,
            coreConfig = coreConfig,
            modelClient = TextAgentModelClient(
                client = model.client,
                enableThinking = false,
            ),
            maximumJobCount = maximumJobCount,
        )
    }

    private suspend fun ensureConfiguredModel() {
        if (!modelService.isPlaceholder) return
        error("No model has been explicitly activated for Agent execution")
    }

    private val applicationContext = context.applicationContext
}
