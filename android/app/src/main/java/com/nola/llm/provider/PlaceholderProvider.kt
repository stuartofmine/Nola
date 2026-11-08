package com.nola.llm.provider

import android.content.Context
import com.nola.R
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.llm.LlmClient
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.time.Duration

@Singleton
class PlaceholderProvider @Inject constructor(
    @ApplicationContext private val context: Context
) : LlmClient {

    private val readyMessage: String
        get() = context.getString(R.string.chat_placeholder_ready)

    private val configureModelMessage: String
        get() = context.getString(R.string.chat_placeholder_configure_model)

    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        return ModelOutput(
            text = readyMessage,
        )
    }

    override fun generateSummaryStream(input: ModelInput): Flow<String> = flow {
        emit(readyMessage)
    }

    override suspend fun chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Boolean,
        timeout: Duration?
    ): String {
        return configureModelMessage
    }
}
