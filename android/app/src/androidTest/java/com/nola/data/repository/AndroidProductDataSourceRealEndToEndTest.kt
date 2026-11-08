package com.nola.data.repository

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.core.AgentCoreContextPrepareBudget
import com.nola.core.AndroidContextCoordinator
import com.nola.core.AndroidContextRequest
import com.nola.core.AndroidContextRoute
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.security.SecureTokenStore
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ChatMemoryProfile
import com.nola.domain.model.UploadedContent
import com.nola.llm.provider.OpenAiProvider
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.serialization.kotlinx.json.json
import java.io.File
import java.net.InetAddress
import java.net.ServerSocket
import java.util.UUID
import kotlin.concurrent.thread
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidProductDataSourceRealEndToEndTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private lateinit var store: MemoryProfileDataStore
    private lateinit var runtime: AndroidApplicationRuntime

    @org.junit.After
    fun closeRuntime() { if (::runtime.isInitialized) runtime.close() }

    @Before
    fun setUp() = runBlocking {
        runtime = AndroidApplicationRuntime(context)
        store = MemoryProfileDataStore(context, SecureTokenStore(context), runtime)
        store.clearAll()
    }

    @Test
    fun assistantSettingsAndIdentitySurviveCanonicalStoreReopen() = runBlocking {
        val principal = AndroidApplicationRuntime(context).use { application ->
        val first = MemoryProfileDataStore(context, SecureTokenStore(context), application)
        first.saveMemoryEnabled(false)
        first.saveResponseStyle("precise")
        first.saveAutoCompactTriggerPercent(90)
        first.saveAgentLlmConfig("https://models.example/v1", "test-secret", "qwen-local")
        first.getOrCreateResourcePrincipalId()
        }

        AndroidApplicationRuntime(context).use { application ->
        val reopened = MemoryProfileDataStore(context, SecureTokenStore(context), application)
        assertEquals(false, reopened.memoryEnabled.first())
        assertEquals("precise", reopened.responseStyle.first())
        assertEquals(90, reopened.autoCompactTriggerPercent.first())
        assertEquals("https://models.example/v1", reopened.agentLlmEndpoint.first())
        assertEquals("qwen-local", reopened.agentLlmModel.first())
        assertEquals("test-secret", reopened.agentLlmApiKey.first())
        assertEquals(principal, reopened.getOrCreateResourcePrincipalId())
        }
    }

    @Test
    fun invalidCurrentSettingsFailInsteadOfFallingBackToDefaults() = runBlocking {
        AndroidApplicationRuntime(context).use { canonical ->
            canonical.putProductSetting(
                "chat.preferences",
                """{"schemaVersion":0,"autoCompactTriggerPercent":80}""",
                System.currentTimeMillis(),
            )
        }
        assertFalse(runCatching { store.autoCompactTriggerPercent.first() }.isSuccess)

        AndroidApplicationRuntime(context).use { canonical ->
            canonical.putProductSetting(
                "assistant.response_style",
                """{"schemaVersion":1,"value":"casual","legacyValue":"precise"}""",
                System.currentTimeMillis() + 1,
            )
        }
        assertFalse(runCatching { store.responseStyle.first() }.isSuccess)
    }

    @Test
    fun modelParametersSurviveCanonicalModelProfileColdReopen() = runBlocking {
        val database = File(context.cacheDir, "model-parameters-${UUID.randomUUID()}.db")
        try {
            AndroidApplicationRuntime(context, database).use { firstStore ->
                ModelParametersStore(firstStore).saveParameters(
                    "qwen3.5-4b",
                    ModelParametersStore.defaultParameters.copy(
                        temperature = 0.23f,
                        maxContextTokens = 131_072,
                        responseStyle = "professional",
                    ),
                )
            }
            AndroidApplicationRuntime(context, database).use { reopenedStore ->
                val reopened = ModelParametersStore(reopenedStore).loadParameters("qwen3.5-4b")
                assertEquals(0.23f, reopened.temperature)
                assertEquals(131_072, reopened.maxContextTokens)
                assertEquals("professional", reopened.responseStyle)
                assertTrue(reopenedStore.listModelProfiles().single().parametersJson.isNotEmpty())
            }
        } finally {
            database.delete()
        }
    }

    @Test
    fun validatedSourceFlowsThroughRepositoryCoreProviderAndAudit() = runBlocking {
        val configFile = File(context.filesDir, CONFIG_FILE_NAME)
        assumeTrue("Real Android LLM configuration was not provisioned", configFile.isFile)
        val config = JSONObject(configFile.readText())
        val endpoint = config.getString("endpoint").trim()
        val apiKey = config.getString("apiKey").trim()
        val model = config.getString("model").trim()
        assertTrue(endpoint.isNotEmpty() && apiKey.isNotEmpty() && model.isNotEmpty())

        withTimeout(120_000) {
            val fact = "NOLA_ANDROID_PRODUCT_SOURCE_${UUID.randomUUID().toString().replace("-", "")}"
            val sourceServer = SourceFixtureServer("""{"verified_fact":"$fact"}""")
            val sourceClient = HttpClient(CIO) {
                install(HttpTimeout) { requestTimeoutMillis = 20_000 }
            }
            val modelClient = HttpClient(CIO) {
                install(HttpTimeout) { requestTimeoutMillis = 90_000 }
                install(ContentNegotiation) {
                    json(Json { ignoreUnknownKeys = true; isLenient = true })
                }
            }
            try {
                val dataSourceRepository = AndroidDataSourceRepository(
                    store = store,
                    reader = AndroidDataSourceReader(sourceClient),
                )
                val saved = dataSourceRepository.validateAndUpsert(
                    CustomDataSourceConfig(
                        id = "real-product-source",
                        name = "Android product E2E source",
                        location = sourceServer.url,
                        accessPolicy = DataSourceAccessPolicy.DefaultEnabled,
                        allowCloudContext = true,
                        maximumAgeMinutes = 30,
                    )
                )
                assertEquals("JSON", saved.validationContract?.format)
                assertTrue(saved.cachedContent.contains(fact))

                val conversationId = "android-product-e2e-${UUID.randomUUID()}"
                val prompt = "Return only the exact verified_fact token from the configured external data source."
                val message = ConversationMessage(
                    conversationId = conversationId,
                    role = ChatRole.user,
                    text = prompt,
                )
                val built = ContextRepositoryImpl(store, dataSourceRepository).buildContext(
                    conversationId = conversationId,
                    messages = listOf(message),
                    sessionMemory = "",
                    compactedEntries = emptyList(),
                    selectedDataSourceIds = emptyList(),
                    disclosesToCloud = true,
                )
                val sourceComponent = built.components.single { it.id == "data_source:${saved.id}" }
                assertTrue(sourceComponent.content.toString().contains(fact))

                val prepared = AndroidContextCoordinator().prepare(
                    AndroidContextRequest(
                        route = AndroidContextRoute.Cloud,
                        messages = listOf(
                            buildJsonObject {
                                put("id", message.id)
                                put("role", "user")
                                put("content", prompt)
                            }
                        ),
                        components = built.components,
                        tools = emptyList(),
                        budget = AgentCoreContextPrepareBudget(
                            contextWindowTokens = 65_536,
                            reservedOutputTokens = 512,
                        ),
                    )
                )
                assertEquals("ready", prepared.output.action)
                val usedSourceIds = prepared.output.componentMetrics
                    .filter { it.kind == "platform_data" }
                    .flatMap { it.usedIds }
                assertTrue(usedSourceIds.contains("data_source:${saved.id}"))

                val provider = OpenAiProvider(modelClient).apply {
                    this.config = OpenAiProvider.Config(
                        baseUrl = endpoint,
                        apiKey = apiKey,
                        model = model,
                        temperature = 0f,
                        maxCompletionTokens = 512,
                    )
                }
                val chunks = provider.generateSummaryStream(prepared.modelInput).toList()
                assertTrue("The real model returned no stream chunks", chunks.isNotEmpty())
                val answer = chunks.joinToString("").trim()
                assertTrue("The real model did not use product-prepared source context: $answer", answer.contains(fact))

                val records = buildDataSourceUsageRecords(
                    usedComponentIds = usedSourceIds,
                    sources = dataSourceRepository.snapshot().first,
                    conversationId = conversationId,
                    consumer = "Android cloud model",
                    disclosedToCloud = true,
                )
                dataSourceRepository.recordUsage(records)
                val persistedSnapshot = dataSourceRepository.snapshot()
                val persisted = persistedSnapshot.second
                assertEquals(1, records.size)
                assertTrue(persisted.any { it.sourceId == saved.id && it.conversationId == conversationId })
                writePersistenceAudit(
                    "data-source",
                    mapOf(
                        "source_definition_persisted" to persistedSnapshot.first.any { it.id == saved.id },
                        "source_cached_content_persisted" to persistedSnapshot.first.any {
                            it.id == saved.id && it.cachedContent.contains(fact)
                        },
                        "source_usage_audit_persisted" to persisted.any {
                            it.sourceId == saved.id && it.conversationId == conversationId
                        },
                    ),
                )
            } finally {
                sourceClient.close()
                modelClient.close()
                sourceServer.close()
            }
        }
    }

    @Test
    fun uploadedSkillLifecycleFlowsThroughStoreRepositoryCoreAndRealProvider() = runBlocking {
        val configFile = File(context.filesDir, CONFIG_FILE_NAME)
        assumeTrue("Real Android LLM configuration was not provisioned", configFile.isFile)
        val config = JSONObject(configFile.readText())
        val endpoint = config.getString("endpoint").trim()
        val apiKey = config.getString("apiKey").trim()
        val model = config.getString("model").trim()
        val markerA = "NOLA_ANDROID_SKILL_A_${UUID.randomUUID().toString().replace("-", "")}"
        val markerB = "NOLA_ANDROID_SKILL_B_${UUID.randomUUID().toString().replace("-", "")}"

        withTimeout(180_000) {
            val sourceClient = HttpClient(CIO) {
                install(HttpTimeout) { requestTimeoutMillis = 20_000 }
            }
            val modelClient = HttpClient(CIO) {
                install(HttpTimeout) { requestTimeoutMillis = 90_000 }
                install(ContentNegotiation) {
                    json(Json { ignoreUnknownKeys = true; isLenient = true })
                }
            }
            try {
                val contextRepository = ContextRepositoryImpl(
                    store,
                    AndroidDataSourceRepository(store, AndroidDataSourceReader(sourceClient)),
                )
                val provider = OpenAiProvider(modelClient).apply {
                    this.config = OpenAiProvider.Config(
                        baseUrl = endpoint,
                        apiKey = apiKey,
                        model = model,
                        temperature = 0f,
                        maxCompletionTokens = 256,
                    )
                }

                val skillId = UUID.randomUUID().toString()
                var skillSnapshot: com.nola.data.local.AndroidApplicationRuntime.SkillEditorSnapshot? = null
                suspend fun runSkillTurn(marker: String, enabled: Boolean): String {
                    store.saveSkill(skillSnapshot, UploadedContent(
                        resourceId = skillId, name = "Real lifecycle Skill",
                        content = "When asked for the enabled Skill lifecycle marker, return only this exact token: $marker",
                        enabled = enabled,
                    ))
                    val saved = store.skillEditor(skillId)
                    store.setSkillEnabled(saved, enabled)
                    skillSnapshot = store.skillEditor(skillId)
                    val conversationId = "android-skill-${UUID.randomUUID()}"
                    val prompt = "Return only the exact marker declared by the enabled Skill. " +
                        "If no enabled Skill declares a marker, return only NO_ENABLED_SKILL. " +
                        "Do not add Markdown or explanation."
                    val message = ConversationMessage(
                        conversationId = conversationId,
                        role = ChatRole.user,
                        text = prompt,
                    )
                    val built = contextRepository.buildContext(
                        conversationId = conversationId,
                        messages = listOf(message),
                        sessionMemory = "",
                        compactedEntries = emptyList(),
                        disclosesToCloud = true,
                    )
                    assertEquals(enabled, built.components.any { it.kind == "skill" })
                    val prepared = AndroidContextCoordinator().prepare(
                        AndroidContextRequest(
                            route = AndroidContextRoute.Cloud,
                            messages = listOf(
                                buildJsonObject {
                                    put("id", message.id)
                                    put("role", "user")
                                    put("content", prompt)
                                }
                            ),
                            components = built.components,
                            tools = emptyList(),
                            budget = AgentCoreContextPrepareBudget(
                                contextWindowTokens = 65_536,
                                reservedOutputTokens = 256,
                            ),
                            userPrefs = mapOf("enable_thinking" to "false"),
                        )
                    )
                    assertEquals(
                        enabled,
                        prepared.output.componentMetrics
                            .filter { it.kind == "skill" }
                            .flatMap { it.usedIds }
                            .isNotEmpty(),
                    )
                    return provider.generateSummaryStream(prepared.modelInput)
                        .toList()
                        .joinToString("")
                        .trim('`', '\'', '"', ' ', '\n', '\r', '\t')
                }

                assertEquals(markerA, runSkillTurn(markerA, enabled = true))
                val edited = runSkillTurn(markerB, enabled = true)
                assertEquals(markerB, edited)
                assertTrue(!edited.contains(markerA))
                val disabled = runSkillTurn(markerB, enabled = false)
                assertEquals("NO_ENABLED_SKILL", disabled)
                assertTrue(!disabled.contains(markerB))
                writePersistenceAudit(
                    "skill",
                    mapOf(
                        "enabled_skill_loaded_from_store" to (edited == markerB),
                        "edited_skill_replaced_previous_content" to !edited.contains(markerA),
                        "disabled_skill_excluded_after_store_update" to (
                            disabled == "NO_ENABLED_SKILL" && !disabled.contains(markerB)
                        ),
                    ),
                )
            } finally {
                sourceClient.close()
                modelClient.close()
            }
        }
    }

    private class SourceFixtureServer(private val body: String) : AutoCloseable {
        private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        private val worker = thread(name = "nola-android-source-fixture", isDaemon = true) {
            server.accept().use { socket ->
                val reader = socket.getInputStream().bufferedReader()
                while (reader.readLine()?.isNotEmpty() == true) Unit
                val payload = body.toByteArray(Charsets.UTF_8)
                socket.getOutputStream().apply {
                    write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ${payload.size}\r\nConnection: close\r\n\r\n".toByteArray())
                    write(payload)
                    flush()
                }
            }
        }

        val url: String = "http://127.0.0.1:${server.localPort}/context.json"

        override fun close() {
            runCatching { server.close() }
            worker.join(2_000)
        }
    }

    private fun writePersistenceAudit(name: String, assertions: Map<String, Boolean>) {
        val assertionObject = JSONObject()
        assertions.forEach { (key, value) -> assertionObject.put(key, value) }
        File(context.filesDir, "nola-real-$name-persistence-audit.json").writeText(
            JSONObject()
                .put("passed", assertions.isNotEmpty() && assertions.values.all { it })
                .put("assertions", assertionObject)
                .toString(2)
        )
    }

    private companion object {
        const val CONFIG_FILE_NAME = "nola-real-llm-e2e.json"
    }
}
