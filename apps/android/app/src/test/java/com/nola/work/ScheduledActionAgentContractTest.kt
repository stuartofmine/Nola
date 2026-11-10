package com.nola.work

import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.DataSourceDocumentInspector
import org.junit.Assert.assertEquals
import org.junit.Test

class ScheduledActionAgentContractTest {
    @Test
    fun configuredSourceBecomesTypedReferenceContext() {
        val action = ScheduledActionEntity(
            id = "action-1",
            localUserId = "user-1",
            name = "Daily revenue",
            url = "https://example.com/revenue.csv",
            format = "csv",
            userPrompt = "Summarize material changes",
        )
        val source = DataSourceDocumentInspector.inspect(
            sourceId = "scheduled_source:action-1",
            location = action.url,
            contentType = "text/csv",
            rawContent = "day,revenue\nMonday,42",
            maximumAgeMinutes = 60,
            now = 1_700_000_000_000,
        )

        val component = scheduledActionReferenceComponent(action, source, "execution-1")

        assertEquals("reference", component.kind)
        assertEquals("scheduled_source:action-1:execution-1", component.id)
        assertEquals(action.url, component.source?.uri)
        assertEquals("text/csv", component.source?.mimeType)
        assertEquals("authorized", component.source?.authorization)
    }

    @Test
    fun scheduledSourceUsesExistingValidatedReaderContract() {
        val action = ScheduledActionEntity(
            id = "action-2",
            localUserId = "user-1",
            name = "Feed",
            url = "https://example.com/feed.json",
            format = "json",
        )

        val source = action.toDataSourceConfig()

        assertEquals("scheduled_source:action-2", source.id)
        assertEquals(action.url, source.location)
        assertEquals(60, source.maximumAgeMinutes)
    }
}
