package com.nola.data.repository

import com.nola.core.AgentCoreDataSourceResolution
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.DataSourceValidationContract
import com.nola.domain.model.DataSourceValidationState
import com.nola.domain.model.DATA_SOURCE_MAXIMUM_AGE_MINUTES
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidDataSourceRepositoryTest {
    @Test
    fun readerPerformsRealHttpExchangeAndBuildsReusableJsonContract() = runBlocking {
        val client = HttpClient(MockEngine { request ->
            assertEquals("https://example.test/live.json", request.url.toString())
            respond(
                content = """[{"city":"Hangzhou","signal":7429,"api_key":"secret-value"}]""",
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, "application/json")
            )
        })
        val result = AndroidDataSourceReader(client).read(
            CustomDataSourceConfig(
                id = "weather",
                name = "City signal",
                location = "https://example.test/live.json",
                maximumAgeMinutes = 15
            )
        )

        assertEquals("JSON", result.contract.format)
        assertEquals(listOf("api_key", "city", "signal"), result.contract.schemaFields)
        assertEquals(1, result.contract.recordCount)
        assertEquals(15, result.contract.maximumAgeMinutes)
        assertTrue(result.contract.readerDescription.contains("JSON"))
        assertTrue(result.contract.redactedSample.contains("[REDACTED]"))
        assertFalse(result.contract.redactedSample.contains("secret-value"))
        assertTrue(result.content.contains("7429"))
    }

    @Test
    fun inspectorExtractsReadableHtmlInsteadOfInjectingMarkup() {
        val result = DataSourceDocumentInspector.inspect(
            sourceId = "news",
            location = "https://example.test/news",
            contentType = "text/html",
            rawContent = "<html><style>.x{}</style><body><h1>Market update</h1><script>steal()</script><p>Signal 9137</p></body></html>",
            maximumAgeMinutes = 60,
            now = 100
        )

        assertEquals("HTML text", result.contract.format)
        assertTrue(result.content.contains("Market update"))
        assertTrue(result.content.contains("Signal 9137"))
        assertFalse(result.content.contains("<h1>"))
        assertFalse(result.content.contains("steal()"))
    }

    @Test
    fun inspectorClampsFreshnessToTheSharedCoreMaximum() {
        val result = DataSourceDocumentInspector.inspect(
            sourceId = "legacy",
            location = "https://example.test/legacy",
            contentType = "text/plain",
            rawContent = "fresh content",
            maximumAgeMinutes = 43_200,
            now = 100
        )

        assertEquals(DATA_SOURCE_MAXIMUM_AGE_MINUTES, result.contract.maximumAgeMinutes)
    }

    @Test
    fun policyAndCloudGateSelectOnlySourcesAllowedForThisRequest() {
        fun source(
            id: String,
            policy: DataSourceAccessPolicy,
            cloud: Boolean = true
        ) = CustomDataSourceConfig(
            id = id,
            name = id,
            location = "https://example.test/$id",
            accessPolicy = policy,
            allowCloudContext = cloud,
            validationContract = DataSourceValidationContract(
                sourceId = id,
                readerDescription = "HTTP GET -> JSON -> RawEvent",
                format = "JSON",
                recordCount = 1,
                validationState = DataSourceValidationState.Verified,
                readRecipe = com.nola.domain.model.DataSourceReadRecipe(
                    method = com.nola.domain.model.DataSourceReadMethod.HttpGet,
                    operationId = "android.http.get",
                    configurationKeys = listOf("url", "maximumBytes"),
                    outputSchemaJson = "{\"type\":\"object\"}"
                ),
                validationDigest = "core-sealed-test"
            ),
            cachedContent = "{\"id\":\"$id\"}"
        )
        val sources = listOf(
            source("default", DataSourceAccessPolicy.DefaultEnabled),
            source("chosen", DataSourceAccessPolicy.ConversationOptIn),
            source("ask", DataSourceAccessPolicy.AskWhenRelevant),
            source("blocked", DataSourceAccessPolicy.Blocked),
            source("local", DataSourceAccessPolicy.DefaultEnabled, cloud = false)
        )

        assertEquals(
            listOf("default", "chosen", "ask", "local"),
            selectUsableDataSources(
                sources,
                listOf("chosen", "ask"),
                disclosesToCloud = false,
                resolve = { source, _ -> coreProjection(source.id != "blocked") }
            ).map { it.id }
        )
        assertEquals(
            listOf("default", "chosen", "ask"),
            selectUsableDataSources(
                sources,
                listOf("chosen", "ask"),
                disclosesToCloud = true,
                resolve = { source, _ -> coreProjection(source.id != "blocked") }
            ).map { it.id }
        )
    }

    @Test
    fun coreUsedComponentIdsBecomeRequestBoundaryAuditRecords() {
        val source = CustomDataSourceConfig(
            id = "pulse",
            name = "Project pulse",
            location = "https://example.test/pulse",
            validationContract = DataSourceValidationContract(
                sourceId = "pulse",
                readerDescription = "HTTP GET -> JSON -> RawEvent",
                format = "JSON",
                recordCount = 1,
                readRecipe = com.nola.domain.model.DataSourceReadRecipe(
                    method = com.nola.domain.model.DataSourceReadMethod.HttpGet,
                    operationId = "android.http.get",
                    configurationKeys = listOf("url", "maximumBytes"),
                    outputSchemaJson = "{\"type\":\"object\"}"
                ),
                validationDigest = "core-sealed-test"
            ),
            cachedContent = "{\"signal\":7429}"
        )
        val records = buildDataSourceUsageRecords(
            usedComponentIds = listOf("platform-data-health", "data_source:pulse", "data_source:pulse"),
            sources = listOf(source),
            conversationId = "conversation-1",
            consumer = "Android cloud model",
            disclosedToCloud = true,
            usedAt = 99
        )

        assertEquals(1, records.size)
        assertEquals("pulse", records.single().sourceId)
        assertEquals("conversation-1", records.single().conversationId)
        assertTrue(records.single().disclosedToCloud)
        assertEquals(99, records.single().usedAt)
    }

    @Test
    fun refreshDeadlineIsEnforcedAndFailedSourcesAreNotUsable() {
        val source = CustomDataSourceConfig(
            id = "deadline",
            name = "Deadline",
            location = "https://example.test/deadline",
            maximumAgeMinutes = 30,
            validationContract = DataSourceValidationContract(
                sourceId = "deadline",
                readerDescription = "HTTP GET -> JSON -> RawEvent",
                format = "JSON",
                recordCount = 1,
                readRecipe = com.nola.domain.model.DataSourceReadRecipe(
                    method = com.nola.domain.model.DataSourceReadMethod.HttpGet,
                    operationId = "android.http.get",
                    configurationKeys = listOf("url", "maximumBytes"),
                    outputSchemaJson = "{\"type\":\"object\"}"
                ),
                validationDigest = "core-sealed-test"
            ),
            cachedContent = "{\"ok\":true}",
            lastSuccessfulReadAt = 1_000
        )

        assertFalse(source.needsRefresh(now = 1_000 + 29 * 60_000L))
        assertTrue(source.needsRefresh(now = 1_000 + 30 * 60_000L))
        assertTrue(source.isUsable)
        assertFalse(
            source.copy(
                validationContract = source.validationContract?.copy(
                    readRecipe = null,
                    validationDigest = null
                )
            ).isUsable
        )
        assertFalse(source.copy(lastError = "network unavailable").isUsable)
    }

    @Test
    fun oversizedFreshnessCannotOutliveTheSharedCoreMaximum() {
        val source = CustomDataSourceConfig(
            id = "oversized-deadline",
            name = "Oversized deadline",
            location = "https://example.test/oversized-deadline",
            maximumAgeMinutes = 43_200,
            lastSuccessfulReadAt = 1_000
        )

        assertFalse(source.needsRefresh(now = 1_000 + (DATA_SOURCE_MAXIMUM_AGE_MINUTES - 1) * 60_000L))
        assertTrue(source.needsRefresh(now = 1_000 + DATA_SOURCE_MAXIMUM_AGE_MINUTES * 60_000L))
    }

    private fun coreProjection(usable: Boolean) = AgentCoreDataSourceResolution(
        schemaVersion = 2,
        enabled = usable,
        usable = usable,
        shouldRequestConsent = false,
        shouldRefresh = false,
        reason = if (usable) "ready" else "blocked"
    )
}
