package com.nola.data.local.db.dao

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import com.nola.data.local.db.entity.ExecutionResultEntity
import kotlinx.coroutines.flow.Flow

@Dao
interface ExecutionResultDao {
    @Query("SELECT * FROM execution_results ORDER BY createdAt DESC LIMIT :limit")
    suspend fun getRecent(limit: Int = 50): List<ExecutionResultEntity>

    @Query("SELECT * FROM execution_results ORDER BY createdAt DESC LIMIT :limit")
    fun observeRecent(limit: Int = 50): Flow<List<ExecutionResultEntity>>

    @Query("""SELECT r.* FROM scheduled_actions AS a JOIN execution_results AS r
        ON r.id = (SELECT other.id FROM execution_results AS other
            WHERE other.actionId = a.id ORDER BY other.createdAt DESC, other.id DESC LIMIT 1)""")
    fun observeLatestPerAction(): Flow<List<ExecutionResultEntity>>

    @Query("SELECT * FROM execution_results WHERE actionId = :actionId ORDER BY createdAt DESC")
    suspend fun getByActionId(actionId: String): List<ExecutionResultEntity>

    @Query("SELECT * FROM execution_results WHERE failureReason = 'permission_required' AND status = 'needs_attention'")
    suspend fun getPermissionWaits(): List<ExecutionResultEntity>

    @Query("SELECT * FROM execution_results WHERE id = :id")
    suspend fun getById(id: String): ExecutionResultEntity?

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(result: ExecutionResultEntity)
}
