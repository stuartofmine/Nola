// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
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
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaDisclosureChevron
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberReduceMotionEnabled
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
    val reduceMotion = rememberReduceMotionEnabled()
    var query by rememberSaveable { mutableStateOf("") }
    val normalizedQuery = query.trim()
    val allConversations = state.conversations.orEmpty()
    val visibleConversations = if (normalizedQuery.isEmpty()) {
        allConversations
    } else {
        allConversations.filter { conv ->
            conv.title.orEmpty().contains(normalizedQuery, ignoreCase = true) ||
                conv.knowledgePoints.contains(normalizedQuery, ignoreCase = true)
        }
    }
    LazyColumn(Modifier.fillMaxSize().testTag("information.knowledge"),
        contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)) {
        item {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.workspace_knowledge), Modifier.weight(1f),
                    style = MaterialTheme.typography.headlineSmall)
                TextButton(onClick = viewModel::refresh, enabled = !state.loading,
                    modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.common_refresh)) }
            }
        }
        if (allConversations.isNotEmpty()) {
            item {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = { Text(stringResource(R.string.knowledge_search_hint)) },
                    leadingIcon = { Icon(painterResource(R.drawable.nola_search), contentDescription = null, modifier = Modifier.size(20.dp)) },
                    trailingIcon = if (query.isNotEmpty()) { {
                        IconButton(onClick = { query = "" }) {
                            Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.notifications_clear_search))
                        }
                    } } else null,
                    shape = RoundedCornerShape(12.dp),
                    colors = OutlinedTextFieldDefaults.colors(
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                        unfocusedBorderColor = Color.Transparent,
                        focusedBorderColor = MaterialTheme.colorScheme.primary,
                    ),
                )
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
        if (state.conversations != null && visibleConversations.isEmpty() && !state.failed && !state.loading) item {
            val isSearching = normalizedQuery.isNotEmpty()
            NolaEmptyState(
                icon = if (isSearching) Icons.Filled.Search else Icons.AutoMirrored.Filled.MenuBook,
                title = stringResource(if (isSearching) R.string.knowledge_search_empty else R.string.workspace_knowledge_empty),
            )
        }
        items(visibleConversations, key = { it.id }) { conversation ->
            var expanded by rememberSaveable(conversation.id) { mutableStateOf(false) }
            val title = ConversationTitlePresentationPolicy.displayTitle(
                conversation.title,
                conversation.id,
                stringResource(R.string.chat_new_chat),
            )
            NolaCard {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 48.dp)
                        .clickable { expanded = !expanded },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        title,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    NolaDisclosureChevron(expanded = expanded, reduceMotion = reduceMotion)
                }
                if (!expanded) {
                    Text(
                        conversation.knowledgePoints.replace(Regex("\\s+"), " ").trim(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                AnimatedVisibility(
                    visible = expanded,
                    enter = nolaDisclosureEnter(reduceMotion),
                    exit = nolaDisclosureExit(reduceMotion),
                ) {
                    SelectionContainer { MarkdownRenderer(conversation.knowledgePoints, Modifier.fillMaxWidth()) }
                }
                TextButton(
                    onClick = { onOpenConversation(conversation.id) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("knowledge.open.${conversation.id}"),
                ) {
                    Text(stringResource(R.string.workspace_knowledge_open))
                }
            }
        }
    }
}
