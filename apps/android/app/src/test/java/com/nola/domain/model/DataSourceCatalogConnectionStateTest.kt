package com.nola.domain.model

import org.junit.Assert.assertEquals
import org.junit.Test

/** Default selection must not masquerade as connectivity or hide a failed read. */
class DataSourceCatalogConnectionStateTest {
    @Test
    fun defaultDisabledValidatedSourceRemainsConnectedForConversationOptIn() {
        val source = verifiedSource().copy(enabled = false)
        val item = catalog(source)

        assertEquals(UnifiedDataSourceConnectionState.Ready, item.connectionState)
        assertEquals("data_source:source", item.id)
        assertEquals("source", item.metadata["customSourceId"])
        assertEquals(DataSourceAccessPolicy.ConversationOptIn.name, item.metadata["accessPolicy"])
    }

    @Test
    fun changingDefaultDoesNotChangeConnectionState() {
        val source = verifiedSource()
        assertEquals(catalog(source).connectionState, catalog(source.copy(enabled = false)).connectionState)
    }

    @Test
    fun failedReadCannotAppearReadyBecauseAnOlderContractExists() {
        val source = verifiedSource().copy(lastError = "request timed out", cachedContent = "old content")
        assertEquals(UnifiedDataSourceConnectionState.Error, catalog(source).connectionState)
        assertEquals(UnifiedDataSourceConnectionState.Error, catalog(source.copy(enabled = false)).connectionState)
    }

    @Test
    fun missingAndFailedValidationAreUnavailableRegardlessOfDefault() {
        val source = verifiedSource()
        for (enabled in listOf(true, false)) {
            assertEquals(
                UnifiedDataSourceConnectionState.Unavailable,
                catalog(source.copy(enabled = enabled, validationContract = null)).connectionState
            )
            assertEquals(
                UnifiedDataSourceConnectionState.Unavailable,
                catalog(source.copy(
                    enabled = enabled,
                    validationContract = source.validationContract!!.copy(validationState = DataSourceValidationState.Failed)
                )).connectionState
            )
        }
    }

    private fun catalog(source: CustomDataSourceConfig): UnifiedDataSource =
        DataSourceCatalogBuilder.forChatContext(
            uploadedDocuments = emptyList(),
            hostUploadedDocuments = emptyList(),
            mcpServers = emptyList(),
            hostMcpServers = emptyList(),
            includeHealthSummary = false,
            customDataSources = listOf(source)
        ).single()

    private fun verifiedSource() = CustomDataSourceConfig(
        id = "source",
        name = "Source",
        location = "https://example.invalid/data",
        validationContract = DataSourceValidationContract(
            sourceId = "source",
            readerDescription = "HTTP",
            format = "text",
            readRecipe = DataSourceReadRecipe(
                method = DataSourceReadMethod.HttpGet,
                operationId = "android.http.get",
                outputSchemaJson = "{\"type\":\"string\"}"
            ),
            validationDigest = "catalog-fixture"
        )
    )
}
