package com.nola.ui.screen.models

import android.content.Context
import android.os.SystemClock
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.R
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.repository.ModelRepository
import com.nola.data.repository.ApiModelConfig
import com.nola.data.remote.api.normalizeCertificateSha256
import com.nola.data.remote.api.isAllowedWebHostTransport
import com.nola.data.remote.dto.MessagingStatusDto
import com.nola.domain.model.ModelWireProtocol
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelParameters
import com.nola.llm.ModelService
import com.nola.llm.local.DeviceModelRecord
import com.nola.llm.local.DeviceModelStore
import com.nola.llm.local.DeviceModelCatalog
import com.nola.llm.local.DeviceModelCatalogEntry
import com.nola.llm.provider.LocalLlamaProvider
import com.nola.llm.provider.LocalModelRuntimeState
import com.nola.llm.provider.OpenAiProvider
import com.nola.llm.provider.WebAgentProvider
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.Job
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import javax.inject.Inject

data class ModelsUiState(
    val connectionInventory: com.nola.data.repository.ModelConnectionInventory? = null,
    val connectionInventoryLoading: Boolean = false,
    val connectionInventoryFailed: Boolean = false,
    val officialModels: List<ModelInfo> = emptyList(),
    val configuredApiModels: List<ModelInfo> = emptyList(),
    val installedLocalModels: List<ModelInfo> = emptyList(),
    val deviceCatalog: List<DeviceModelCatalogEntry> = DeviceModelCatalog.entries,
    val downloadingModelId: String? = null,
    val downloadProgress: Float? = null,
    val webHostModel: ModelInfo? = null,
    val isLoading: Boolean = false,
    val selectedModelId: String? = null,
    val selectedModelSource: ModelInfo.ModelSource? = null,
    val testingConnection: Boolean = false,
    val connectionTestResult: String? = null,
    val connectionTestSucceeded: Boolean? = null,
    val activatingModelId: String? = null,
    val activatingModelSource: ModelInfo.ModelSource? = null,
    val apiEndpoint: String = "",
    val apiKey: String = "",
    val savingEndpoint: Boolean = false,
    val endpointSaveError: String? = null,
    val apiModel: String = "",
    val apiWireProtocol: ModelWireProtocol = ModelWireProtocol.ChatCompletions,
    val webHostUrl: String = "",
    val webHostToken: String = "",
    val webHostCertificateSha256: String = "",
    val hostMessaging: MessagingStatusDto? = null,
    val openingHostManagement: Boolean = false,
    val showParameters: Boolean = false,
    val parameterTarget: ModelInfo? = null,
    val parameterSnapshot: ModelParametersStore.ParameterSnapshot? = null,
    val savingParameters: Boolean = false,
    val parameterEditorVersion: Long = 0,
    val parameterError: String? = null,
    val parameters: ModelParameters = ModelParameters(),
    val runningLocalModelId: String? = null,
    val modelTestTarget: ModelInfo? = null,
    val modelTestPrompt: String = "",
    val modelTestOutput: String = "",
    val modelTestRunning: Boolean = false,
    val modelTestFirstTokenMs: Long? = null,
    val modelTestTotalMs: Long? = null,
    val modelTestError: String? = null,
)

internal fun ModelsUiState.acceptParameterReload(
    baseline: ModelParametersStore.ParameterSnapshot,
    refreshed: ModelParametersStore.ParameterSnapshot,
): ModelsUiState = if (parameterSnapshot !== baseline) this else copy(
    parameters = refreshed.parameters,
    parameterSnapshot = refreshed,
    parameterEditorVersion = parameterEditorVersion + 1,
    parameterError = null,
    savingParameters = false,
)

internal fun modelTestCloudConfiguration(
    endpoint: ApiModelConfig,
    model: ModelInfo,
    parameters: ModelParameters,
): OpenAiProvider.Config = OpenAiProvider.Config(
    baseUrl = endpoint.endpoint,
    apiKey = endpoint.apiKey,
    model = model.id,
    wireProtocol = endpoint.wireProtocol,
    executionCapabilities = model.executionCapabilities,
    temperature = parameters.temperature,
    topP = parameters.topP,
    topK = parameters.topK,
    sendTopK = false,
    maxCompletionTokens = parameters.maxCompletionTokens,
).withParameters(parameters).withResolvedRuntime(endpoint.resolvedRuntime)

@HiltViewModel
class ModelsViewModel @Inject constructor(
    private val modelRepo: ModelRepository,
    private val modelService: ModelService,
    private val modelParametersStore: ModelParametersStore,
    private val prefs: PreferenceDataStore,
    private val openAiProvider: OpenAiProvider,
    private val localLlamaProvider: LocalLlamaProvider,
    private val deviceModelStore: DeviceModelStore,
    private val webAgentProvider: WebAgentProvider,
    private val webHostApi: com.nola.data.remote.api.WebHostApiService,
    private val applicationRuntime: com.nola.data.local.AndroidApplicationRuntime,
    @ApplicationContext private val context: Context
) : ViewModel() {
    private val connections = com.nola.data.repository.ModelConnectionRepository(applicationRuntime)

    // Keep credentials in memory while the user visits another tab.
    internal val connectionDraft = EndpointConfigDraft("", "", "", ModelWireProtocol.ChatCompletions)
    private val _uiState = MutableStateFlow(ModelsUiState())
    val uiState: StateFlow<ModelsUiState> = _uiState.asStateFlow()
    private var installedLocalRecords: List<DeviceModelRecord> = emptyList()
    private var modelTestJob: Job? = null
    private var modelTestGeneration = 0L

    init {
        loadConnectionInventory()
        loadModels()
        viewModelScope.launch {
            localLlamaProvider.state.collect { runtime ->
                _uiState.update {
                    it.copy(
                        runningLocalModelId = when (runtime) {
                            is LocalModelRuntimeState.Ready -> runtime.modelId
                            is LocalModelRuntimeState.Generating -> runtime.modelId
                            else -> null
                        }
                    )
                }
            }
        }
    }

    fun loadConnectionInventory() {
        if (_uiState.value.connectionInventoryLoading) return
        _uiState.update { it.copy(connectionInventoryLoading = true, connectionInventoryFailed = false) }
        viewModelScope.launch {
            try {
                val inventory = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { connections.load() }
                _uiState.update { it.copy(connectionInventory = inventory) }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { _uiState.update { it.copy(connectionInventoryFailed = true) } }
            finally { _uiState.update { it.copy(connectionInventoryLoading = false) } }
        }
    }

    private var modelInventoryJob: Job? = null

    fun loadModels() {
        modelInventoryJob?.cancel()
        modelInventoryJob = viewModelScope.launch {
            _uiState.update { it.copy(isLoading = true) }

            try {
                val apiConfig = try {
                    modelRepo.getApiModelConfig()
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    // An unconfirmed model must remain visible so its capabilities can be reviewed.
                    publishStatus(error.message ?: context.getString(R.string.models_endpoint_save_failed), false)
                    null
                }
                val host = prefs.webHostConfiguration()
                val webHostUrl = host?.address.orEmpty()
                val webHostToken = host?.token.orEmpty()
                val webHostCertificateSha256 = host?.pin.orEmpty()
                val hasWebHostConfig = host != null
                val hostSettings = if (hasWebHostConfig) try {
                    withTimeoutOrNull(3_000) {
                        webHostApi.getAgentSettings(
                            hostBaseUrl = webHostUrl,
                            token = webHostToken,
                            pinnedCertificateSha256 = webHostCertificateSha256
                        )
                    }
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { null } else null

                installedLocalRecords = deviceModelStore.installedModels()
                val configuredModels = modelRepo.listConfiguredModels()
                val savedDefault = prefs.defaultModelSelection(configuredModels + installedLocalRecords.map {
                    ModelInfo(it.id, it.displayName, ModelInfo.ModelSource.Local)
                } + if (hasWebHostConfig) listOf(ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost)) else emptyList())
                _uiState.update {
                    it.copy(
                        configuredApiModels = configuredModels,
                        installedLocalModels = installedLocalRecords.map { record ->
                            ModelInfo(id = record.id, name = record.displayName, source = ModelInfo.ModelSource.Local)
                        },
                        apiEndpoint = apiConfig?.endpoint.orEmpty(),
                        apiKey = apiConfig?.apiKey.orEmpty(),
                        apiModel = apiConfig?.model.orEmpty(),
                        apiWireProtocol = apiConfig?.wireProtocol ?: ModelWireProtocol.ChatCompletions,
                        webHostModel = if (hasWebHostConfig) {
                            ModelInfo(id = "web-host", name = "Nola Web Host", source = ModelInfo.ModelSource.WebHost)
                        } else {
                            null
                        },
                        webHostUrl = webHostUrl,
                        webHostToken = webHostToken,
                        webHostCertificateSha256 = webHostCertificateSha256,
                        hostMessaging = hostSettings?.messaging,
                        selectedModelId = if (modelService.isPlaceholder) savedDefault?.selectionId else modelService.currentProfileId ?: modelService.currentModelId,
                        selectedModelSource = if (modelService.isPlaceholder) savedDefault?.source else modelService.currentModelSource.toModelSource()
                    )
                }

                val official = withTimeoutOrNull(3_000) {
                    modelRepo.listOfficialModels()
                }.orEmpty()
                _uiState.update { it.copy(officialModels = official) }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                publishStatus(error.message ?: context.getString(R.string.models_endpoint_save_failed), false)
            } finally {
                _uiState.update { it.copy(isLoading = false) }
            }
        }
    }

    fun selectModel(model: ModelInfo) {
        viewModelScope.launch {
            _uiState.update {
                it.copy(
                    activatingModelId = model.selectionId,
                    activatingModelSource = model.source,
                    connectionTestResult = null,
                    connectionTestSucceeded = null
                )
            }
            try {
                when (model.source) {
                    ModelInfo.ModelSource.Cloud -> {
                        val config = modelRepo.getApiModelConfigFor(model)
                        if (config == null) {
                            publishStatus(context.getString(R.string.models_error_configure_endpoint_before_cloud), false)
                            return@launch
                        }
                        configureOpenAiProvider(config, model, "cloud")
                    }
                    ModelInfo.ModelSource.Local -> {
                        val ready = localLlamaProvider.state.value as? LocalModelRuntimeState.Ready
                        if (ready?.modelId != model.id) {
                            publishStatus(context.getString(R.string.models_error_model_unavailable), false)
                            return@launch
                        }
                        modelService.reconfigure(localLlamaProvider, model.id, "local")
                    }
                    ModelInfo.ModelSource.WebHost -> {
                        if (!configureWebHostProvider()) {
                            publishStatus(context.getString(R.string.models_error_web_host_config_required), false)
                            return@launch
                        }
                    }
                }
                prefs.saveDefaultModelSelection(model)
                _uiState.update {
                    it.copy(
                        selectedModelId = model.selectionId,
                        selectedModelSource = model.source,
                        endpointSaveError = null,
                        connectionTestResult = context.getString(R.string.models_model_activated_format, model.name),
                        connectionTestSucceeded = true
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                publishStatus(context.getString(R.string.models_test_failed_format, e.message ?: ""), false)
            } finally {
                _uiState.update { it.copy(activatingModelId = null, activatingModelSource = null) }
            }
        }
    }

    fun testConnection() {
        viewModelScope.launch {
            _uiState.update {
                it.copy(testingConnection = true, connectionTestResult = null, connectionTestSucceeded = null)
            }
            try {
                val endpoint = _uiState.value.apiEndpoint
                val apiKey = _uiState.value.apiKey

                if (endpoint.isBlank()) {
                    publishStatus(context.getString(R.string.models_error_no_endpoint_configured), false)
                    return@launch
                }

                val modelCount = openAiProvider.testConnection(endpoint, apiKey)
                publishStatus(context.getString(R.string.models_test_ok_format, modelCount), true)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                publishStatus(context.getString(R.string.models_test_failed_format, e.message ?: ""), false)
            } finally {
                _uiState.update { it.copy(testingConnection = false) }
            }
        }
    }

    fun clearConnectionStatus() {
        _uiState.update {
            it.copy(connectionTestResult = null, connectionTestSucceeded = null)
        }
    }

    fun openModelTest(model: ModelInfo) {
        if (model.source == ModelInfo.ModelSource.WebHost) return
        modelTestGeneration++
        modelTestJob?.cancel()
        modelTestJob = null
        _uiState.update { it.copy(
            modelTestTarget = model,
            modelTestPrompt = context.getString(R.string.models_inference_test_default_prompt),
            modelTestOutput = "",
            modelTestRunning = false,
            modelTestFirstTokenMs = null,
            modelTestTotalMs = null,
            modelTestError = null,
        ) }
    }

    fun updateModelTestPrompt(prompt: String) {
        _uiState.update { it.copy(modelTestPrompt = prompt) }
    }

    fun stopModelTest() {
        modelTestGeneration++
        modelTestJob?.cancel()
        modelTestJob = null
        _uiState.update { it.copy(modelTestRunning = false) }
    }

    fun closeModelTest() {
        modelTestGeneration++
        modelTestJob?.cancel()
        modelTestJob = null
        _uiState.update { it.copy(modelTestTarget = null, modelTestRunning = false) }
    }

    fun runModelTest() {
        val state = _uiState.value
        val model = state.modelTestTarget ?: return
        val prompt = state.modelTestPrompt.trim()
        if (state.modelTestRunning || prompt.isEmpty()) return
        val generation = ++modelTestGeneration
        _uiState.update { it.copy(
            modelTestRunning = true,
            modelTestOutput = "",
            modelTestFirstTokenMs = null,
            modelTestTotalMs = null,
            modelTestError = null,
        ) }
        modelTestJob = viewModelScope.launch {
            val startedAt = SystemClock.elapsedRealtime()
            try {
                val parameters = modelParametersStore.loadParameters(model.parameterProfileId)
                val client = when (model.source) {
                    ModelInfo.ModelSource.Cloud -> {
                        val config = modelRepo.getApiModelConfigFor(model, parameters)
                            ?: error(context.getString(R.string.models_error_configure_endpoint_before_cloud))
                        openAiProvider.clientForTurn(modelTestCloudConfiguration(config, model, parameters))
                    }
                    ModelInfo.ModelSource.Local -> {
                        val ready = localLlamaProvider.state.value as? LocalModelRuntimeState.Ready
                        if (ready?.modelId != model.id) {
                            error(context.getString(R.string.models_inference_test_start_local))
                        }
                        localLlamaProvider
                    }
                    ModelInfo.ModelSource.WebHost -> return@launch
                }
                client.generateSummaryStream(ModelInput(
                    messages = listOf(mapOf("role" to "user", "content" to prompt)),
                    systemPrompt = parameters.customSystemPrompt.takeIf(String::isNotBlank),
                )).collect { chunk ->
                    if (chunk.isNotEmpty()) {
                        _uiState.update { current ->
                            if (modelTestGeneration != generation || current.modelTestTarget != model) current else current.copy(
                                modelTestOutput = current.modelTestOutput + chunk,
                                modelTestFirstTokenMs = current.modelTestFirstTokenMs
                                    ?: SystemClock.elapsedRealtime() - startedAt,
                            )
                        }
                    }
                }
                _uiState.update { current ->
                    if (modelTestGeneration != generation || current.modelTestTarget != model) current else current.copy(
                        modelTestTotalMs = SystemClock.elapsedRealtime() - startedAt,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update { current ->
                    if (modelTestGeneration != generation || current.modelTestTarget != model) current else current.copy(
                        modelTestError = error.localizedMessage ?: error::class.java.simpleName,
                        modelTestTotalMs = SystemClock.elapsedRealtime() - startedAt,
                    )
                }
            } finally {
                _uiState.update { current ->
                    if (modelTestGeneration != generation || current.modelTestTarget != model) current else current.copy(modelTestRunning = false)
                }
            }
        }
    }

    fun saveApiEndpoint(endpoint: String, apiKey: String, model: String, wireProtocol: ModelWireProtocol, onSaved: () -> Unit = {}) {
        if (_uiState.value.savingEndpoint) return
        _uiState.update { it.copy(savingEndpoint = true, endpointSaveError = null) }
        viewModelScope.launch {
            var savedModels: List<ModelInfo>? = null
            try {
                val config = ApiModelConfig(endpoint, apiKey, model, wireProtocol)
                modelRepo.saveApiModelConfig(config)
                val configuredModels = modelRepo.listConfiguredModels()
                savedModels = configuredModels
                val configured = requireNotNull(configuredModels.firstOrNull {
                    it.id == model && modelRepo.getApiModelConfigFor(it)?.endpoint == endpoint
                }) { "The saved model was not published" }
                configureOpenAiProvider(config, configured, "cloud")
                prefs.saveDefaultModelSelection(configured)
                _uiState.update {
                    it.copy(
                        apiEndpoint = endpoint,
                        apiKey = apiKey,
                        apiModel = model,
                        apiWireProtocol = wireProtocol,
                        configuredApiModels = configuredModels,
                        selectedModelId = configured.selectionId,
                        selectedModelSource = configured.source,
                        connectionTestResult = context.getString(R.string.models_endpoint_saved),
                        connectionTestSucceeded = true
                    )
                }
                onSaved()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                _uiState.update {
                    it.copy(
                        configuredApiModels = savedModels ?: it.configuredApiModels,
                        endpointSaveError = context.getString(if (savedModels != null) R.string.models_saved_not_activated else R.string.models_endpoint_save_failed),
                        connectionTestResult = context.getString(if (savedModels != null) R.string.models_saved_not_activated else R.string.models_endpoint_save_failed),
                        connectionTestSucceeded = false
                    )
                }
            } finally {
                _uiState.update { it.copy(savingEndpoint = false) }
            }
        }
    }

    fun saveExecutionCapabilities(
        model: ModelInfo,
        capabilities: com.nola.domain.model.ModelExecutionCapabilities,
        onResult: (String?) -> Unit,
    ) {
        viewModelScope.launch {
            val error = try {
                modelRepo.updateModelExecutionCapabilities(model, capabilities)
                val refreshed = modelRepo.listConfiguredModels()
                _uiState.update { it.copy(configuredApiModels = refreshed) }
                if (model.selectionId == modelService.currentProfileId) {
                    openAiProvider.config = openAiProvider.config?.copy(executionCapabilities = capabilities)
                }
                null
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                failure.localizedMessage ?: context.getString(R.string.models_config_save_failed)
            }
            onResult(error)
        }
    }

    fun startLocalModel(model: ModelInfo) {
        val record = installedLocalRecords.firstOrNull { it.id == model.id } ?: return
        viewModelScope.launch {
            _uiState.update { it.copy(activatingModelId = model.id, activatingModelSource = model.source) }
            try {
                val params = modelParametersStore.loadLocalParameters(model.id, record.contextWindow)
                localLlamaProvider.start(record, params)
                modelService.reconfigure(localLlamaProvider, model.id, "local")
                prefs.saveDefaultModelSelection(model)
                _uiState.update {
                    it.copy(
                        selectedModelId = model.selectionId,
                        selectedModelSource = ModelInfo.ModelSource.Local,
                        runningLocalModelId = model.id,
                        connectionTestResult = context.getString(R.string.models_model_activated_format, model.name),
                        connectionTestSucceeded = true,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                publishStatus(context.getString(R.string.models_test_failed_format, error.message.orEmpty()), false)
            } finally {
                _uiState.update { it.copy(activatingModelId = null, activatingModelSource = null) }
            }
        }
    }

    fun stopLocalModel() {
        viewModelScope.launch {
            localLlamaProvider.stopAsync()
            if (_uiState.value.selectedModelSource == ModelInfo.ModelSource.Local) {
                modelService.usePlaceholder()
                _uiState.update { it.copy(selectedModelId = null, selectedModelSource = null) }
            }
        }
    }

    fun deleteModel(model: ModelInfo) {
        if (model.source == ModelInfo.ModelSource.Local) {
            deleteLocalModel(model)
            return
        }
        if (model.source != ModelInfo.ModelSource.Cloud) return
        val profileId = model.profileId ?: model.id
        viewModelScope.launch {
            try {
                applicationRuntime.deleteModelProfile(profileId, System.currentTimeMillis())
                val configuredModels = modelRepo.listConfiguredModels()
                val wasSelected = _uiState.value.selectedModelId == model.selectionId &&
                    _uiState.value.selectedModelSource == ModelInfo.ModelSource.Cloud
                if (wasSelected) {
                    modelService.usePlaceholder()
                    prefs.clearDefaultModelSelection()
                }
                _uiState.update {
                    it.copy(
                        configuredApiModels = configuredModels,
                        selectedModelId = if (wasSelected) null else it.selectedModelId,
                        selectedModelSource = if (wasSelected) null else it.selectedModelSource,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                publishStatus(context.getString(R.string.models_delete_failed_format, error.message.orEmpty()), false)
            }
        }
    }

    fun deleteLocalModel(model: ModelInfo) {
        viewModelScope.launch {
            try {
                deviceModelStore.delete(model.id, _uiState.value.runningLocalModelId)
                loadModels()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                publishStatus(context.getString(R.string.models_delete_failed_format, error.message.orEmpty()), false)
            }
        }
    }

    private var localDownloadJob: Job? = null

    fun cancelLocalModelDownload() {
        localDownloadJob?.cancel()
        localDownloadJob = null
    }

    fun downloadLocalModel(entry: DeviceModelCatalogEntry) {
        if (_uiState.value.downloadingModelId != null) return
        localDownloadJob = viewModelScope.launch {
            _uiState.update {
                it.copy(
                    downloadingModelId = entry.id,
                    downloadProgress = 0f,
                    connectionTestResult = null,
                    connectionTestSucceeded = null,
                )
            }
            try {
                deviceModelStore.install(
                    id = entry.id,
                    displayName = entry.displayName,
                    sourceUrl = entry.downloadUrl,
                    expectedSha256 = entry.sha256,
                    contextWindow = entry.contextWindow,
                    recommendedRamBytes = entry.recommendedRamBytes,
                ) { progress ->
                    val total = progress.totalBytes
                    _uiState.update { state ->
                        state.copy(downloadProgress = total?.let { progress.downloadedBytes.toFloat() / it.toFloat() })
                    }
                }
                loadModels()
                publishStatus(context.getString(R.string.models_download_complete_format, entry.displayName), true)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                publishStatus(context.getString(R.string.models_download_failed_format, error.message.orEmpty()), false)
            } finally {
                localDownloadJob = null
                _uiState.update { it.copy(downloadingModelId = null, downloadProgress = null) }
            }
        }
    }

    private var webHostConnectionJob: Job? = null

    fun cancelWebHostConnection() { webHostConnectionJob?.cancel(); webHostConnectionJob = null }

    fun disconnectWebHost() {
        cancelWebHostConnection()
        viewModelScope.launch {
            try {
                prefs.clearWebHostConfig()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                publishStatus(webHostConnectionMessage(context, error), false)
                return@launch
            }
            modelInventoryJob?.cancel()
            val wasSelected = _uiState.value.selectedModelSource == ModelInfo.ModelSource.WebHost
            if (modelService.currentModelSource == "web-host") modelService.usePlaceholder()
            webAgentProvider.config = null
            _uiState.update {
                it.copy(
                    webHostUrl = "",
                    webHostToken = "",
                    webHostCertificateSha256 = "",
                    webHostModel = null,
                    hostMessaging = null,
                    connectionTestResult = null,
                    connectionTestSucceeded = null,
                    selectedModelId = if (wasSelected) null else it.selectedModelId,
                    selectedModelSource = if (wasSelected) null else it.selectedModelSource,
                )
            }
            loadModels()
        }
    }

    fun saveWebHostConfig(
        url: String,
        token: String,
        certificateSha256: String,
        importedHostIdentity: String? = null,
        onComplete: (Boolean, String) -> Unit = { _, _ -> }
    ) {
        if (url.isBlank() || token.isBlank()) {
            val message = context.getString(R.string.models_error_web_host_config_required)
            publishStatus(message, false)
            onComplete(false, message)
            return
        }
        val normalizedPin = normalizeCertificateSha256(certificateSha256)
        if (normalizedPin == null) {
            val message = context.getString(R.string.models_error_invalid_certificate_pin)
            publishStatus(message, false)
            onComplete(false, message)
            return
        }
        if (!isAllowedWebHostTransport(url)) {
            val message = context.getString(R.string.models_error_public_http_forbidden)
            publishStatus(message, false)
            onComplete(false, message)
            return
        }
        if (normalizedPin.isNotEmpty() && !url.trim().startsWith("https://", ignoreCase = true)) {
            val message = context.getString(R.string.models_error_certificate_pin_requires_https)
            publishStatus(message, false)
            onComplete(false, message)
            return
        }

        webHostConnectionJob?.cancel()
        webHostConnectionJob = viewModelScope.launch {
            try {
                val normalizedUrl = com.nola.data.remote.api.normalizeWebHostBaseUrl(url)
                val savedOrigin = prefs.webHostBaseUrl.first()
                val previousIdentity = if (sameWebHostDestination(savedOrigin, normalizedUrl)) prefs.webHostIdentity.first() else ""
                val settings = kotlinx.coroutines.withTimeout(15_000) { webHostApi.getAgentSettings(
                        hostBaseUrl = normalizedUrl,
                        token = token.trim(),
                        pinnedCertificateSha256 = normalizedPin
                    ) }
                val identity = verifyWebHostConnection(previousIdentity, importedHostIdentity) { settings }
                prefs.saveWebHostConfig(url, token, normalizedPin, verifiedHostId = identity)
                val saved = checkNotNull(prefs.pairedWebHost())
                modelInventoryJob?.cancel()
                val previous = webAgentProvider.config
                if (previous != null && previous.expectedHostId == saved.identity &&
                    previous.hostBaseUrl == saved.address && previous.pinnedCertificateSha256 == saved.pin) {
                    webAgentProvider.config = previous.copy(token = saved.token)
                } else {
                    webAgentProvider.config = null
                    if (modelService.currentModelSource == "web-host") modelService.usePlaceholder()
                }
                val hostModel = ModelInfo(id = "web-host", name = "Nola Web Host", source = ModelInfo.ModelSource.WebHost)
                val hostIsDefault = prefs.defaultModelSelection(listOf(hostModel)) != null
                val successMessage = context.getString(if (hostIsDefault) R.string.models_web_host_default_verified else R.string.models_web_host_verified)
                _uiState.update {
                    it.copy(
                        webHostUrl = saved.address,
                        webHostToken = saved.token,
                        webHostCertificateSha256 = saved.pin,
                        selectedModelId = if (!hostIsDefault && it.selectedModelSource == ModelInfo.ModelSource.WebHost) null else it.selectedModelId,
                        selectedModelSource = if (!hostIsDefault && it.selectedModelSource == ModelInfo.ModelSource.WebHost) null else it.selectedModelSource,
                        webHostModel = hostModel,
                        hostMessaging = settings.messaging,
                        connectionTestResult = successMessage,
                        connectionTestSucceeded = true
                    )
                }
                loadModels()
                onComplete(true, successMessage)
            } catch (e: kotlinx.coroutines.TimeoutCancellationException) {
                val message = context.getString(R.string.models_error_host_timeout)
                publishStatus(message, false)
                onComplete(false, message)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (e: Exception) {
                val message = webHostConnectionMessage(context, e)
                publishStatus(message, false)
                onComplete(false, message)
            }
        }
    }

    fun openParameters(model: ModelInfo) {
        viewModelScope.launch {
            val snapshot = try {
                if (model.source == ModelInfo.ModelSource.Local) {
                    val contextWindow = installedLocalRecords.firstOrNull { it.id == model.id }?.contextWindow ?: 4096
                    modelParametersStore.localParameterSnapshot(model.id, model.name, contextWindow)
                } else {
                    modelParametersStore.parameterSnapshot(model.parameterProfileId)
                }
            } catch (error: Exception) {
                publishStatus(error.localizedMessage ?: context.getString(R.string.models_error_model_unavailable), false)
                return@launch
            }
            _uiState.update {
                it.copy(
                    showParameters = true,
                    parameterTarget = model,
                    parameters = snapshot.parameters,
                    parameterSnapshot = snapshot,
                    parameterError = null,
                    savingParameters = false,
                    parameterEditorVersion = it.parameterEditorVersion + 1,
                )
            }
        }
    }

    fun openHostMessagingManagement(
        target: String,
        onReady: (String) -> Unit
    ) {
        val state = _uiState.value
        if (state.openingHostManagement || state.webHostUrl.isBlank() || state.webHostToken.isBlank()) return
        _uiState.update { it.copy(openingHostManagement = true) }
        viewModelScope.launch {
            try {
                val paired = prefs.pairedWebHost() ?: return@launch
                val url = webHostApi.requestWebManagementUrl(
                    hostBaseUrl = paired.address,
                    token = paired.token,
                    target = target,
                    pinnedCertificateSha256 = paired.pin
                )
                val current = prefs.pairedWebHost()
                if (current != null && paired.identity == current.identity && paired.address == current.address &&
                    paired.pin == current.pin && paired.token == current.token) onReady(url)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                publishStatus(webHostConnectionMessage(context, error), false)
            } finally {
                _uiState.update { it.copy(openingHostManagement = false) }
            }
        }
    }

    fun closeParameters() {
        _uiState.update { it.copy(showParameters = false, parameterTarget = null, parameterSnapshot = null, parameterError = null) }
    }

    fun reloadParameters() {
        val current = _uiState.value
        val target = current.parameterTarget ?: return
        val baseline = current.parameterSnapshot ?: return
        if (current.savingParameters) return
        _uiState.update { it.copy(savingParameters = true) }
        viewModelScope.launch {
            try {
                val refreshed = if (target.source == ModelInfo.ModelSource.Local) {
                    val contextWindow = installedLocalRecords.firstOrNull { it.id == target.id }?.contextWindow ?: 4096
                    modelParametersStore.localParameterSnapshot(target.id, target.name, contextWindow)
                } else {
                    modelParametersStore.parameterSnapshot(target.parameterProfileId)
                }
                _uiState.update { state ->
                    state.acceptParameterReload(baseline, refreshed)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update { state ->
                    if (state.parameterSnapshot !== baseline) state else state.copy(
                        parameterError = error.localizedMessage ?: context.getString(R.string.models_error_model_unavailable),
                    )
                }
            } finally {
                _uiState.update { state -> if (state.parameterSnapshot !== baseline) state else state.copy(savingParameters = false) }
            }
        }
    }

    fun updateParameters(params: ModelParameters) {
        val target = _uiState.value.parameterTarget ?: return
        val snapshot = _uiState.value.parameterSnapshot ?: return
        if (_uiState.value.savingParameters) return
        _uiState.update { it.copy(savingParameters = true, parameterError = null) }
        viewModelScope.launch {
            try {
                modelParametersStore.saveParameters(snapshot, params)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                if (_uiState.value.parameterSnapshot === snapshot) {
                    _uiState.update { it.copy(parameterError = error.localizedMessage
                        ?: context.getString(R.string.models_error_model_unavailable)) }
                }
                return@launch
            } finally {
                if (_uiState.value.parameterSnapshot === snapshot) {
                    _uiState.update { it.copy(savingParameters = false) }
                }
            }
            if (_uiState.value.parameterSnapshot !== snapshot) return@launch
            _uiState.update { it.copy(parameters = params) }
            closeParameters()
            when (target.source) {
                ModelInfo.ModelSource.Cloud -> {
                    if (target.id == modelService.currentModelId && target.profileId == modelService.currentProfileId) {
                        openAiProvider.config = openAiProvider.config?.withParameters(params)
                    }
                }
                ModelInfo.ModelSource.Local -> {
                    if (_uiState.value.runningLocalModelId == target.id) {
                        val record = installedLocalRecords.firstOrNull { it.id == target.id }
                        if (record != null) {
                            val clamped = params.copy(
                                maxContextTokens = params.maxContextTokens.coerceIn(512, record.contextWindow.coerceAtLeast(512))
                            )
                            try {
                                localLlamaProvider.start(record, clamped)
                            } catch (cancelled: CancellationException) {
                                throw cancelled
                            } catch (error: Throwable) {
                                publishStatus(context.getString(R.string.models_test_failed_format, error.message.orEmpty()), false)
                            }
                        }
                    }
                }
                ModelInfo.ModelSource.WebHost -> Unit
            }
        }
    }

    private suspend fun configureOpenAiProvider(
        config: ApiModelConfig,
        model: ModelInfo,
        source: String
    ) {
        val params = modelParametersStore.loadParameters(model.parameterProfileId)
        openAiProvider.config = OpenAiProvider.Config(
            baseUrl = config.endpoint,
            apiKey = config.apiKey,
            model = model.id,
            wireProtocol = config.wireProtocol,
            executionCapabilities = model.executionCapabilities,
            temperature = params.temperature,
            topP = params.topP,
            topK = params.topK,
            sendTopK = false,
            maxCompletionTokens = params.maxCompletionTokens
        ).withParameters(params).withResolvedRuntime(config.resolvedRuntime)
        modelService.reconfigure(openAiProvider, model.id, source, model.profileId)
    }

    private suspend fun configureWebHostProvider(): Boolean {
        val host = prefs.pairedWebHost() ?: return false
        webAgentProvider.config = WebAgentProvider.Config(
            hostBaseUrl = host.address,
            token = host.token,
            maxIterations = 0,
            expectedHostId = host.identity,
            clientId = prefs.getOrCreateClientUuid(),
            pinnedCertificateSha256 = host.pin
        )
        modelService.reconfigure(webAgentProvider, "web-host", "web-host")
        return true
    }

    private fun publishStatus(message: String, succeeded: Boolean) {
        _uiState.update {
            it.copy(connectionTestResult = message, connectionTestSucceeded = succeeded)
        }
    }

    private fun String.toModelSource(): ModelInfo.ModelSource? {
        return when (this) {
            "local" -> ModelInfo.ModelSource.Local
            "cloud" -> ModelInfo.ModelSource.Cloud
            "web-host" -> ModelInfo.ModelSource.WebHost
            else -> null
        }
    }
}
