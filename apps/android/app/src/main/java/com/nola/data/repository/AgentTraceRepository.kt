package com.nola.data.repository

import com.nola.agent.trace.AgentTraceEvent
import com.nola.agent.trace.AgentTraceProjection
import com.nola.agent.trace.AgentTraceReducer
import com.nola.data.local.db.dao.AgentTraceDao
import com.nola.data.local.db.entity.AgentTraceEventEntity
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

@Singleton
@OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)
class AgentTraceRepository @Inject constructor(
    private val dao: AgentTraceDao,
) {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    suspend fun append(event: AgentTraceEvent): Boolean =
        dao.append(event.toEntity(json)) != -1L

    suspend fun events(traceId: String): List<AgentTraceEvent> =
        dao.events(traceId).mapNotNull { entity ->
            runCatching { json.decodeFromString<AgentTraceEvent>(entity.eventJson) }.getOrNull()
        }

    suspend fun replay(traceId: String): AgentTraceProjection =
        AgentTraceReducer.replay(events(traceId))

    suspend fun latestProjection(conversationId: String): AgentTraceProjection? =
        dao.latestTraceId(conversationId)?.let { replay(it) }
}

private fun AgentTraceEvent.toEntity(json: Json): AgentTraceEventEntity = AgentTraceEventEntity(
    traceId = traceId,
    eventId = eventId,
    runId = runId,
    conversationId = conversationId,
    messageId = messageId,
    turnId = turnId,
    sequence = sequence,
    emittedAt = emittedAt,
    kind = kind,
    eventJson = json.encodeToString(this),
)
