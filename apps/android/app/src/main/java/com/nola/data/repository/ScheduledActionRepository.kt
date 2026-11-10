package com.nola.data.repository

import com.nola.data.local.db.dao.ExecutionResultDao
import com.nola.data.local.db.dao.ScheduledActionDao
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.remote.api.BackendApiService
import kotlinx.coroutines.flow.Flow
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
    suspend fun getAll(): List<ScheduledActionEntity>
    suspend fun getById(id: String): ScheduledActionEntity?
    suspend fun create(action: ScheduledActionEntity)
    suspend fun delete(id: String)
    suspend fun setEnabled(id: String, enabled: Boolean)
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

    override suspend fun getAll(): List<ScheduledActionEntity> = actionDao.getAll()
    override suspend fun getById(id: String): ScheduledActionEntity? = actionDao.getById(id)
    override suspend fun create(action: ScheduledActionEntity) {
        val host = com.nola.core.AndroidAgentPermissionBroker(context).runtimeIdentity("automation-definition").hostId
            ?: error("Automation needs an identified local Host")
        applicationRuntime.saveAutomationDefinition(action.id, action.name, action.enabled, buildJsonObject {
            put("url", action.url); put("format", action.format); put("frequency", action.scheduleFrequency)
            put("scheduled_time", action.scheduledTimeIso); put("weekday", action.scheduledWeekday)
            put("timezone", action.timezone)
        }, AndroidAutomationSpec(requireNotNull(action.userPrompt).trim(), host, AndroidAgentPermissionMode.Cautious.wireValue,
            listOf("web_source" to "scheduled_source:${action.id}"), emptyList()))
        actionDao.upsert(action)
    }
    override suspend fun delete(id: String) {
        applicationRuntime.deleteAutomationDefinition(id)
        actionDao.deleteById(id)
    }
    override suspend fun setEnabled(id: String, enabled: Boolean) {
        val current = actionDao.getById(id) ?: error("Automation is missing")
        create(current.copy(enabled = enabled))
    }
    override suspend fun getResults(actionId: String): List<ExecutionResultEntity> = resultDao.getByActionId(actionId)
    override suspend fun getRecentResults(limit: Int): List<ExecutionResultEntity> = resultDao.getRecent(limit)
    override fun observeRecentResults(limit: Int): Flow<List<ExecutionResultEntity>> = resultDao.observeRecent(limit)
    override suspend fun recordResult(result: ExecutionResultEntity) = resultDao.upsert(result)
}
