package com.nola.data.repository

import androidx.test.core.app.ApplicationProvider
import com.nola.core.AgentCoreNative
import com.nola.core.AgentCoreContextPreparation
import com.nola.core.AgentCoreContextPrepareInput
import com.nola.core.AgentCoreContextPrepareBudget
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.domain.model.CustomDataSourceConfig
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.File
import java.net.InetAddress
import java.net.ServerSocket
import java.net.SocketException
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSourceHttpContextRealTest {
    @Test
    fun explicitlySelectedSourceRefreshesThroughHttpJniAndContextSelection() = runBlocking {
        assertTrue("Real JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "http-context-${System.nanoTime()}")
        root.mkdirs()
        val database = File(root, "product.db")
        val requests = AtomicInteger()
        val serverError = AtomicReference<Throwable?>()
        val beforeResponse = AtomicReference<(() -> Unit)?>(null)
        val server = ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"))
        val location = "http://127.0.0.1:${server.localPort}/source.json"
        val worker = thread(isDaemon = true, name = "source-http-fixture") {
            try {
                while (!server.isClosed) {
                    server.accept().use { socket ->
                        socket.soTimeout = 5_000
                        val input = socket.getInputStream().bufferedReader()
                        check(input.readLine().startsWith("GET /source.json "))
                        while (!input.readLine().isNullOrEmpty()) { }
                        requests.incrementAndGet()
                        beforeResponse.get()?.invoke()
                        val body = "{\"signal\":7429}".toByteArray(Charsets.UTF_8)
                        socket.getOutputStream().apply {
                            write(("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n" +
                                "Content-Length: ${body.size}\r\nConnection: close\r\n\r\n").toByteArray())
                            write(body)
                            flush()
                        }
                    }
                }
            } catch (error: SocketException) {
                if (!server.isClosed) serverError.set(error)
            } catch (error: Throwable) {
                serverError.set(error)
            }
        }
        val http = HttpClient(CIO)
        try {
            AndroidApplicationRuntime(context, database).use { application ->
                val inspected = DataSourceDocumentInspector.inspect(
                    sourceId = "http-source", location = location,
                    contentType = "application/json", rawContent = "{\"signal\":1111}",
                    maximumAgeMinutes = 1, now = System.currentTimeMillis() - 7_200_000
                )
                application.saveDataSource(CustomDataSourceConfig(
                    id = "http-source", name = "HTTP source", location = location,
                    enabled = false, allowCloudContext = false, maximumAgeMinutes = 1,
                    validationContract = inspected.contract, cachedContent = inspected.content
                ))
                val profile = MemoryProfileDataStore(context, SecureTokenStore(context), application)
                val sources = AndroidDataSourceRepository(profile, AndroidDataSourceReader(http))
                beforeResponse.set {
                    val pending = com.nola.data.local.CanonicalDataSourceClient(application::execute).get("http-source")!!
                    assertEquals("pending", pending.readState["status"]!!.jsonPrimitive.content)
                    assertEquals(inspected.contract.validatedAt,
                        java.time.Instant.parse(pending.validation.contract!!["validatedAt"]!!.jsonPrimitive.content).toEpochMilli())
                }
                val contexts = ContextRepositoryImpl(profile, sources)
                val conversations = ConversationRepositoryImpl(application, com.nola.data.local.AndroidAssistantJournal(application), com.nola.llm.local.DeviceModelStore(context))
                val first = conversations.createConversation("First").id
                val second = conversations.createConversation("Second").id
                assertTrue(sources.sourcesForConversation(first, false).isEmpty())
                assertEquals(0, requests.get())
                conversations.setDataSourceEnabled(first, "data_source:http-source", true)
                val selected = sources.sourcesForConversation(first, false).single()
                assertEquals("{\"signal\":7429}", selected.cachedContent)
                assertEquals(inspected.contract.validatedAt, selected.validationContract!!.validatedAt)
                assertEquals(1, requests.get())
                assertFalse(application.listDataSources().single().enabled)
                assertEquals(selected.canonicalRevision, application.listDataSources().single().canonicalRevision)
                assertEquals(selected.cachedContent, sources.sourcesForConversation(first, false).single().cachedContent)
                val assembled = contexts.buildContext(
                    conversationId = first, messages = emptyList(), sessionMemory = "",
                    compactedEntries = emptyList(), selectedDataSourceIds = emptyList(),
                    disclosesToCloud = false
                )
            val dataComponents = assembled.components.filter { it.id == "data_source:http-source" }
            val canonical = com.nola.data.local.CanonicalDataSourceClient(application::execute).get("http-source")!!
            val sourceMetadata = dataComponents.single().source!!
            assertEquals(canonical.authorization.state, sourceMetadata.authorization)
            assertEquals(
                java.time.Instant.ofEpochMilli(canonical.readState.getValue("observed_at_ms").jsonPrimitive.content.toLong()).toString(),
                sourceMetadata.observedAt,
            )
            val prepared = requireNotNull(AgentCoreContextPreparation.prepare(
                AgentCoreContextPrepareInput(
                    components = assembled.components,
                    budget = AgentCoreContextPrepareBudget(contextWindowTokens = 32768, reservedOutputTokens = 2048),
                )
            ))
            assertTrue(prepared.messages.toString().contains("7429"))
            assertTrue(prepared.componentMetrics.any { "data_source:http-source" in it.usedIds })
                assertEquals(1, dataComponents.size)
                assertEquals("platform_data", dataComponents.single().kind)
                assertTrue(dataComponents.single().content.jsonPrimitive.content.contains("\"signal\":7429"))
                assertEquals(1, assembled.contextPack.dataSourceIds.count { it == "data_source:http-source" })
                assertTrue(sources.sourcesForConversation(first, true).isEmpty())
                val cloudContext = contexts.buildContext(
                    conversationId = first, messages = emptyList(), sessionMemory = "",
                    compactedEntries = emptyList(), selectedDataSourceIds = listOf("data_source:http-source"),
                    disclosesToCloud = true
                )
                assertTrue(cloudContext.components.none { it.id == "data_source:http-source" })
                val cloudPrepared = requireNotNull(AgentCoreContextPreparation.prepare(
                    AgentCoreContextPrepareInput(
                        components = cloudContext.components,
                        budget = AgentCoreContextPrepareBudget(contextWindowTokens = 32768, reservedOutputTokens = 2048),
                    )
                ))
                assertFalse(cloudPrepared.messages.toString().contains("7429"))
                assertTrue(cloudPrepared.componentMetrics.none { "data_source:http-source" in it.usedIds })
                assertFalse(cloudContext.contextPack.dataSourceIds.contains("data_source:http-source"))
                assertTrue(sources.sourcesForConversation(second, false).isEmpty())
                conversations.setDataSourceEnabled(first, "data_source:http-source", false)
                assertTrue(sources.sourcesForConversation(first, false).isEmpty())
                val disabledContext = contexts.buildContext(
                    conversationId = first, messages = emptyList(), sessionMemory = "",
                    compactedEntries = emptyList(), selectedDataSourceIds = listOf("data_source:http-source"),
                    disclosesToCloud = false
                )
                assertTrue(disabledContext.components.none { it.id == "data_source:http-source" })
                val disabledPrepared = requireNotNull(AgentCoreContextPreparation.prepare(
                    AgentCoreContextPrepareInput(
                        components = disabledContext.components,
                        budget = AgentCoreContextPrepareBudget(contextWindowTokens = 32768, reservedOutputTokens = 2048),
                    )
                ))
                assertFalse(disabledPrepared.messages.toString().contains("7429"))
                assertTrue(disabledPrepared.componentMetrics.none { "data_source:http-source" in it.usedIds })
                assertFalse(disabledContext.contextPack.dataSourceIds.contains("data_source:http-source"))
                assertEquals(1, requests.get())
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                assertEquals("{\"signal\":7429}", reopened.listDataSources().single().cachedContent)
                assertFalse(reopened.listDataSources().single().enabled)
            }
            assertEquals(null, serverError.get())
        } finally {
            http.close()
            server.close()
            worker.join(6_000)
            root.deleteRecursively()
        }
    }
}
