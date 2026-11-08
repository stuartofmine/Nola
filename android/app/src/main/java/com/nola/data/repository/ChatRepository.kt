package com.nola.data.repository

import com.nola.domain.model.ChatMemoryProfile
import com.nola.data.local.datastore.MemoryProfileDataStore
import kotlinx.coroutines.flow.first
import javax.inject.Inject
import javax.inject.Singleton

interface ChatRepository {
    suspend fun getMemoryProfile(): ChatMemoryProfile
    suspend fun getAgentLlmConfig(): Triple<String, String, String>
}

@Singleton
class ChatRepositoryImpl @Inject constructor(
    private val memoryProfileDataStore: MemoryProfileDataStore
) : ChatRepository {

    override suspend fun getMemoryProfile(): ChatMemoryProfile {
        return memoryProfileDataStore.getSnapshot()
    }

    override suspend fun getAgentLlmConfig(): Triple<String, String, String> {
        return Triple(
            memoryProfileDataStore.agentLlmEndpoint.first(),
            memoryProfileDataStore.agentLlmApiKey.first(),
            memoryProfileDataStore.agentLlmModel.first()
        )
    }
}
