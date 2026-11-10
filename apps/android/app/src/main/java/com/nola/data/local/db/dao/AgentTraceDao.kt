package com.nola.data.local.db.dao

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import com.nola.data.local.db.entity.AgentTraceEventEntity

@Dao
interface AgentTraceDao {
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun append(event: AgentTraceEventEntity): Long

    @Query(
        "SELECT * FROM agent_trace_events " +
            "WHERE traceId = :traceId ORDER BY sequence ASC, emittedAt ASC, eventId ASC"
    )
    suspend fun events(traceId: String): List<AgentTraceEventEntity>

    @Query(
        "SELECT traceId FROM agent_trace_events WHERE conversationId = :conversationId " +
            "ORDER BY rowid DESC LIMIT 1"
    )
    suspend fun latestTraceId(conversationId: String): String?

    @Query("DELETE FROM agent_trace_events WHERE conversationId = :conversationId")
    suspend fun deleteConversationTrace(conversationId: String)
}

