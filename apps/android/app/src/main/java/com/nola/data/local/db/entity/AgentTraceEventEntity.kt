package com.nola.data.local.db.entity

import androidx.room.Entity
import androidx.room.Index

@Entity(
    tableName = "agent_trace_events",
    primaryKeys = ["traceId", "eventId"],
    indices = [
        Index("conversationId"),
        Index("messageId"),
        Index(value = ["traceId", "sequence"], unique = true),
    ],
)
data class AgentTraceEventEntity(
    val traceId: String,
    val eventId: String,
    val runId: String,
    val conversationId: String?,
    val messageId: String?,
    val turnId: String,
    val sequence: Long,
    val emittedAt: String,
    val kind: String,
    val eventJson: String,
)

