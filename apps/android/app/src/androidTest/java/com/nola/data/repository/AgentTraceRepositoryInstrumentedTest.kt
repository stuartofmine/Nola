package com.nola.data.repository

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.agent.trace.AgentTraceEvent
import com.nola.data.local.db.NolaDatabase
import com.nola.data.repository.AgentTraceRepository
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AgentTraceRepositoryInstrumentedTest {
    @Test
    fun sharedCoreControlTracePersistsAndReplaysWithoutOwningMarkdown() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val events = instrumentation.context.assets.open("long_task_trace_v1.json").bufferedReader().use { reader ->
            Json { ignoreUnknownKeys = true }.decodeFromString<List<AgentTraceEvent>>(reader.readText())
        }
        val database = Room.inMemoryDatabaseBuilder(context, NolaDatabase::class.java)
            .allowMainThreadQueries()
            .build()
        val repository = AgentTraceRepository(database.agentTraceDao())

        val appendResults = runBlocking { events.map { repository.append(it) } }
        assertTrue(appendResults.first())
        assertFalse(appendResults[5])
        val projection = runBlocking { repository.replay("trace_contract_long_task") }
        assertEquals(9L, projection.lastSequence)
        assertEquals(2, projection.tools.size)
        assertEquals(8192, projection.usage?.usedTokens)
        assertTrue(projection.isTerminal)
        database.close()
    }

}
