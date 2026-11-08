// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import com.nola.R
import com.nola.data.repository.ConversationRepository
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationTitlePresentationPolicy
import com.nola.ui.components.MarkdownRenderer
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal data class ConversationKnowledgeState(
    val conversations: List<Conversation>? = null,
    val loading: Boolean = false,
    val failed: Boolean = false,
)

@HiltViewModel
internal class ConversationKnowledgeViewModel @Inject constructor(
    private val conversations: ConversationRepository,
) : ViewModel() {
    private val state = MutableStateFlow(ConversationKnowledgeState())
    val uiState = state.asStateFlow()
    init { refresh() }

    fun refresh() {
        if (state.value.loading) return
        state.value = state.value.copy(loading = true, failed = false)
        viewModelScope.launch {
            try {
                val result = withContext(Dispatchers.IO) {
                    conversations.getAllConversations()
                        .filter { it.agentRoomOwner == null && it.knowledgePoints.isNotBlank() }
                        .sortedByDescending { it.lastActiveAt }
                }
                state.value = ConversationKnowledgeState(conversations = result)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { state.value = state.value.copy(loading = false, failed = true) }
        }
    }
}

/** Existing canonical conversation knowledge, read only; editing stays in its original conversation. */
@Composable
internal fun ConversationKnowledgeScreen(
    onOpenConversation: (String) -> Unit,
    viewModel: ConversationKnowledgeViewModel = hiltViewModel(),
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    LazyColumn(Modifier.fillMaxSize().testTag("information.knowledge"),
        contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp)) {
        item {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.workspace_knowledge), Modifier.weight(1f),
                    style = MaterialTheme.typography.headlineSmall)
                TextButton(onClick = viewModel::refresh, enabled = !state.loading,
                    modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.common_refresh)) }
            }
        }
        if (state.loading) item { CircularProgressIndicator() }
        if (state.failed) item {
            Column {
                Text(stringResource(R.string.workspace_knowledge_failed), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = viewModel::refresh, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.common_retry))
                }
            }
        }
        if (state.conversations?.isEmpty() == true && !state.failed && !state.loading) item {
            Text(stringResource(R.string.workspace_knowledge_empty),
                style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        items(state.conversations.orEmpty(), key = { it.id }) { conversation ->
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(ConversationTitlePresentationPolicy.displayTitle(conversation.title, conversation.id,
                    stringResource(R.string.chat_new_chat)), style = MaterialTheme.typography.titleMedium,
                    maxLines = 2, overflow = TextOverflow.Ellipsis)
                SelectionContainer { MarkdownRenderer(conversation.knowledgePoints, Modifier.fillMaxWidth()) }
                TextButton(onClick = { onOpenConversation(conversation.id) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("knowledge.open.${conversation.id}")) {
                    Text(stringResource(R.string.workspace_knowledge_open))
                }
            }
        }
    }
}
