package com.nola.data.repository

import com.nola.domain.model.CustomDataSourceConfig
import kotlinx.coroutines.CancellationException

/** Executes a Host read after the caller has resolved enablement and policy. */
internal suspend fun refreshDataSourceContent(
    source: CustomDataSourceConfig,
    enabledForRead: Boolean,
    read: suspend (CustomDataSourceConfig) -> DataSourceReadResult
): CustomDataSourceConfig {
    if (!enabledForRead || source.validationContract == null) return source
    return try {
        val result = read(source)
        source.copy(
            validationContract = result.contract,
            cachedContent = result.content,
            lastSuccessfulReadAt = result.contract.observedAt,
            lastError = null,
            updatedAt = System.currentTimeMillis()
        )
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        source.copy(
            lastError = error.localizedMessage ?: error.javaClass.simpleName,
            updatedAt = System.currentTimeMillis()
        )
    }
}
