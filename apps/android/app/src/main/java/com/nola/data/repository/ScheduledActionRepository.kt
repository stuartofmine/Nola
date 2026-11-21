package com.nola.data.repository

import com.nola.data.local.db.dao.ExecutionResultDao
import com.nola.data.local.db.dao.ScheduledActionDao
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.remote.api.BackendApiService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import javax.inject.Inject
import javax.inject.Singleton
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.AndroidAutomationSpec
import com.nola.core.AndroidAgentPermissionMode
import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

interface ScheduledActionRepository {
    fun observeAll(): Flow<List<ScheduledActionEntity>>
    fun observeLatestPerAction(): Flow<List<ExecutionResultEntity>>
    suspend fun getAll(): List<ScheduledActionEntity>
    suspend fun getById(id: String): ScheduledActionEntity?
    suspend fun create(action: ScheduledActionEntity)
    suspend fun delete(id: String)
    suspend fun setEnabled(id: String, enabled: Boolean)
    suspend fun getResult(id: String): ExecutionResultEntity?
    suspend fun disableCompletedOccurrence(action: ScheduledActionEntity, definitionRevision: Long)
    suspend fun getResults(actionId: String): List<ExecutionResultEntity>
    suspend fun getRecentResults(limit: Int = 50): List<ExecutionResultEntity>
    fun observeRecentResults(limit: Int = 50): Flow<List<ExecutionResultEntity>>
    suspend fun recordResult(result: ExecutionResultEntity)
}

@Singleton
class ScheduledActionRepositoryImpl @Inject constructor(
    private val actionDao: ScheduledActionDao,
    private val resultDao: ExecutionResultDao,
    private val api: BackendApiService,
    private val applicationRuntime: AndroidApplicationRuntime,
    @ApplicationContext private val context: Context,
) : ScheduledActionRepository {

    override fun observeAll() = actionDao.observeAll()
    override fun observeLatestPerAction() = resultDao.observeLatestPerAction()
    override suspend fun getAll(): List<ScheduledActionEntity> = actionDao.getAll()
    override suspend fun getById(id: String): ScheduledActionEntity? = actionDao.getById(id)
    private val definitionWrites = kotlinx.coroutines.sync.Mutex()

    override suspend fun create(action: ScheduledActionEntity) = definitionWrites.withLock { save(action) }

    private suspend fun save(action: ScheduledActionEntity, expectedRevision: Long? = null) = withContext(Dispatchers.IO) {
        val host = com.nola.core.AndroidAgentPermissionBroker(context).runtimeIdentity("automation-definition").hostId
            ?: error("Automation needs an identified local Host")
        applicationRuntime.saveAutomationDefinition(action.id, action.name, action.enabled, buildJsonObject {
            put("url", action.url); put("format", action.format); put("frequency", action.scheduleFrequency)
            put("scheduled_time", action.scheduledTimeIso); put("weekday", action.scheduledWeekday)
            put("timezone", action.timezone)
        }, AndroidAutomationSpec(requireNotNull(action.userPrompt).trim(), host, AndroidAgentPermissionMode.Cautious.wireValue,
            listOf("web_source" to "scheduled_source:${action.id}"), emptyList()), expectedRevision)
        actionDao.upsert(action)
    }
    override suspend fun delete(id: String) = definitionWrites.withLock {
        withContext(Dispatchers.IO) {
            applicationRuntime.deleteAutomationDefinition(id)
            actionDao.deleteById(id)
        }
    }
    override suspend fun setEnabled(id: String, enabled: Boolean) = definitionWrites.withLock {
        val current = withContext(Dispatchers.IO) { actionDao.getById(id) } ?: error("Automation is missing")
        save(current.copy(enabled = enabled))
    }
    override suspend fun disableCompletedOccurrence(action: ScheduledActionEntity, definitionRevision: Long) {
        definitionWrites.withLock {
            withContext(Dispatchers.IO) {
                if (actionDao.getById(action.id) != action) return@withContext
                val definition = applicationRuntime.loadAutomationDefinition(action.id) ?: return@withContext
                if (definition.revision == definitionRevision) save(action.copy(enabled = false), definitionRevision)
                else if (!definition.enabled) actionDao.setEnabled(action.id, false)
            }
        }
    }
    override suspend fun getResult(id: String) = resultDao.getById(id)
    override suspend fun getResults(actionId: String): List<ExecutionResultEntity> = resultDao.getByActionId(actionId)
    override suspend fun getRecentResults(limit: Int): List<ExecutionResultEntity> = resultDao.getRecent(limit)
    override fun observeRecentResults(limit: Int): Flow<List<ExecutionResultEntity>> = resultDao.observeRecent(limit)
    override suspend fun recordResult(result: ExecutionResultEntity) = resultDao.upsert(result)
}
