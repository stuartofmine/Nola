package com.nola.data.local.db.dao

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import com.nola.data.local.db.entity.ScheduledActionEntity

@Dao
interface ScheduledActionDao {
    @Query("SELECT * FROM scheduled_actions ORDER BY createdAt DESC")
    suspend fun getAll(): List<ScheduledActionEntity>

    @Query("SELECT * FROM scheduled_actions WHERE id = :id")
    suspend fun getById(id: String): ScheduledActionEntity?

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(action: ScheduledActionEntity)

    @Query("DELETE FROM scheduled_actions WHERE id = :id")
    suspend fun deleteById(id: String)

    @Query("UPDATE scheduled_actions SET enabled = :enabled WHERE id = :id")
    suspend fun setEnabled(id: String, enabled: Boolean)
}
