// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.core.AndroidAgentTurnService
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.repository.ModelRepository
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent

/** Debug-only access to real app singletons for owned-device native integration. */
@EntryPoint @InstallIn(SingletonComponent::class)
interface AndroidExpertNativeTestDependencies {
    fun turnService(): AndroidAgentTurnService
    fun modelRepository(): ModelRepository
    fun runtime(): AndroidApplicationRuntime
    fun webAgentProvider(): com.nola.llm.provider.WebAgentProvider
}
