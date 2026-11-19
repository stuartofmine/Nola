package com.nola.agent.context

import com.nola.core.AndroidContextRoute
import com.nola.core.AndroidPreparedContext
import com.nola.data.repository.AndroidDataSourceRepository
import com.nola.data.repository.buildDataSourceUsageRecords
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceUsageRecord
import javax.inject.Inject

class ChatDataSourceUsageRecorder @Inject constructor(
    private val dataSourceRepository: AndroidDataSourceRepository,
) {
    suspend fun record(
        prepared: AndroidPreparedContext,
        conversationId: String,
        route: AndroidContextRoute,
    ) {
        val records = buildUsageRecordsForPrepared(
            prepared = prepared,
            sources = dataSourceRepository.snapshot().first,
            conversationId = conversationId,
            route = route,
        )
        if (records.isEmpty()) return
        dataSourceRepository.recordUsage(records)
    }
}

internal fun buildUsageRecordsForPrepared(
    prepared: AndroidPreparedContext,
    sources: List<CustomDataSourceConfig>,
    conversationId: String,
    route: AndroidContextRoute,
): List<DataSourceUsageRecord> {
    val usedSourceIds = prepared.output.componentMetrics
        .asSequence()
        .filter { it.kind == "platform_data" }
        .flatMap { it.usedIds.asSequence() }
        .mapNotNull { id ->
            id.removePrefix("data_source:").takeIf { it != id && it.isNotBlank() }
        }
        .distinct()
        .toList()
    if (usedSourceIds.isEmpty()) return emptyList()
    return buildDataSourceUsageRecords(
        usedComponentIds = usedSourceIds.map { "data_source:$it" },
        sources = sources,
        conversationId = conversationId,
        consumer = when (route) {
            AndroidContextRoute.Local -> "Android local model"
            AndroidContextRoute.Web -> "Android Web Host"
            AndroidContextRoute.Cloud -> "Android cloud model"
            AndroidContextRoute.PermissionResume,
            AndroidContextRoute.DeviceResume -> "Android resumed agent"
        },
        disclosedToCloud = route != AndroidContextRoute.Local,
    )
}
