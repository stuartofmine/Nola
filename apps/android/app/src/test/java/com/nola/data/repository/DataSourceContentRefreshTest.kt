package com.nola.data.repository

import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceValidationContract
import com.nola.domain.model.DataSourceValidationState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class DataSourceContentRefreshTest {
    private fun source() = CustomDataSourceConfig(
        id = "source", name = "Source", location = "https://example.test/data", enabled = false,
        cachedContent = "previous", canonicalRevision = 9,
        validationContract = DataSourceValidationContract(
            sourceId = "source", readerDescription = "Fixture", format = "text",
            recordCount = 1, validationState = DataSourceValidationState.Verified
        )
    )

    @Test
    fun explicitSelectionReadsDefaultDisabledSourceWithoutChangingItsDefault() = runBlocking {
        val original = source()
        var reads = 0
        val refreshed = refreshDataSourceContent(original, true) {
            reads++
            DataSourceReadResult(content = "current", contract = requireNotNull(it.validationContract))
        }
        assertEquals(1, reads)
        assertEquals("current", refreshed.cachedContent)
        assertFalse(refreshed.enabled)
        assertEquals(original.canonicalRevision, refreshed.canonicalRevision)
    }

    @Test
    fun disabledSelectionDoesNotReadEvenWhenDefaultIsEnabled() = runBlocking {
        val original = source().copy(enabled = true)
        val result = refreshDataSourceContent(original, false) { error("Must not read") }
        assertSame(original, result)
    }

    @Test
    fun cancellationIsNotTurnedIntoPersistableReadFailure() = runBlocking {
        val cancellation = CancellationException("View disposed")
        val result = runCatching {
            refreshDataSourceContent(source(), true) { throw cancellation }
        }
        assertSame(cancellation, result.exceptionOrNull())
    }

    @Test
    fun ordinaryReadFailurePreservesContentButRecordsFailure() = runBlocking {
        val result = refreshDataSourceContent(source(), true) { error("Network unavailable") }
        assertEquals("previous", result.cachedContent)
        assertEquals("Network unavailable", result.lastError)
        assertTrue(result.validationContract != null)
    }
}
