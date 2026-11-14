package com.nola.ui.screen.chat

import androidx.compose.animation.Crossfade
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.material3.FloatingActionButton
import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.withStyle
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.HorizontalDivider
import androidx.compose.foundation.layout.imePadding
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.material3.OutlinedTextField
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nola.ui.components.nolaPageGutter
import com.nola.R
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationTitlePresentationPolicy
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaIconBadge
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import java.text.DateFormat
import java.util.Date
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ConversationListScreen(
    conversations: List<Conversation>,
    currentConversationId: String? = null,
    onConversationSelected: (String) -> Unit,
    onNewConversation: () -> Unit,
    onDeleteConversation: (String) -> Unit,
    search: String,
    onSearchChange: (String) -> Unit,
    listState: LazyListState,
    previews: Map<String, String>,
    drafts: Map<String, String> = emptyMap(),
    activity: Map<String, ChatUiState>,
    experts: Map<String, List<String>> = emptyMap(),
    onLoadPreview: (String) -> Unit,
    error: String? = null,
    onRetry: () -> Unit = {},
    expertChildren: @Composable (Conversation) -> Unit = {}
) {
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    var deleteTarget by remember { mutableStateOf<Conversation?>(null) }
    var needsResponseOnly by remember { mutableStateOf(false) }
    val timeFmt = remember(Locale.getDefault()) { DateFormat.getTimeInstance(DateFormat.SHORT, Locale.getDefault()) }
    val dayFmt = remember(Locale.getDefault()) { DateFormat.getDateInstance(DateFormat.SHORT, Locale.getDefault()) }


    deleteTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = { Text(stringResource(R.string.conversation_delete_title)) },
            text = {
                Text(
                    stringResource(
                        R.string.conversation_delete_message,
                        ConversationTitlePresentationPolicy.displayTitle(
                            target.title,
                            target.id,
                            stringResource(R.string.conversation_untitled),
                        ),
                    )
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    onDeleteConversation(target.id)
                    deleteTarget = null
                }) {
                    Text(stringResource(R.string.common_delete), color = NolaColors.danger)
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteTarget = null }) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        )
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.surface,
        topBar = {
            NolaTopAppBar(title = stringResource(R.string.conversations_title),
                titleStyle = MaterialTheme.typography.headlineSmall.copy(fontSize = 28.sp, lineHeight = 36.sp)) {}
        },
        floatingActionButton = {
            FloatingActionButton(onClick = onNewConversation, containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.testTag("chat.conversations.new")) {
                Icon(painterResource(R.drawable.nola_edit), stringResource(R.string.chat_new_chat))
            }
        }
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding(), horizontalAlignment = Alignment.CenterHorizontally) {
            OutlinedTextField(value = search, onValueChange = onSearchChange,
                placeholder = { Text(stringResource(R.string.conversations_search_title)) }, singleLine = true,
                leadingIcon = { Icon(painterResource(R.drawable.nola_search), contentDescription = null, modifier = Modifier.size(20.dp)) },
                shape = RoundedCornerShape(12.dp),
                colors = OutlinedTextFieldDefaults.colors(unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer, focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer, unfocusedBorderColor = Color.Transparent, focusedBorderColor = MaterialTheme.colorScheme.primary),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus(); keyboard?.hide() }),
                modifier = Modifier.widthIn(max = 720.dp).fillMaxWidth().padding(horizontal = nolaPageGutter(), vertical = 8.dp).testTag("chat.conversations.search"))
            if (error != null) {
                Text(error, Modifier.padding(horizontal = nolaPageGutter(), vertical = 8.dp), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = onRetry) { Text(stringResource(R.string.common_retry)) }
            }
            Row(Modifier.fillMaxWidth().padding(horizontal = nolaPageGutter()), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                listOf(false to R.string.activity_all, true to R.string.conversations_waiting).forEach { (waiting, label) ->
                    TextButton(onClick = { needsResponseOnly = waiting }) {
                        Text(stringResource(label), fontWeight = if (needsResponseOnly == waiting) FontWeight.SemiBold else FontWeight.Normal,
                            color = if (needsResponseOnly == waiting) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            val visible = conversations.filter {
                (!needsResponseOnly || activity[it.id]?.permissionRequest != null) &&
                    (search.isBlank() || it.title.orEmpty().contains(search.trim(), true) || drafts[it.id].orEmpty().contains(search.trim(), true))
            }
            LazyColumn(state = listState, modifier = Modifier.widthIn(max = 720.dp).fillMaxSize().testTag("chat.conversations.list")) {
                if (visible.isEmpty()) item {
                    Text(stringResource(if (conversations.isEmpty()) R.string.conversations_empty_message else R.string.conversations_no_results),
                        modifier = Modifier.padding(24.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                items(visible, key = { it.id }) { conv ->
                    LaunchedEffect(conv.id, conv.lastActiveAt) { onLoadPreview(conv.id) }
                    Row(Modifier.fillMaxWidth().combinedClickable(onClick = { onConversationSelected(conv.id) }, onLongClick = { if (activity[conv.id]?.processingState?.blocksInput != true) deleteTarget = conv })
                        .testTag("chat.conversation.${conv.id}").padding(start = nolaPageGutter(), top = 15.dp, bottom = 16.dp, end = nolaPageGutter()),
                        verticalAlignment = Alignment.Top) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Row(verticalAlignment = Alignment.Top) {
                                Text(ConversationTitlePresentationPolicy.displayTitle(conv.title, conv.id, stringResource(R.string.chat_new_chat)),
                                    modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleMedium.copy(fontSize = 16.sp, lineHeight = 23.sp, fontWeight = FontWeight.Medium),
                                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Spacer(Modifier.width(8.dp))
                                Text((if (android.text.format.DateUtils.isToday(conv.lastActiveAt)) timeFmt else dayFmt).format(Date(conv.lastActiveAt)),
                                    style = MaterialTheme.typography.labelSmall.copy(fontSize = 12.sp, lineHeight = 18.sp),
                                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 3.dp))
                            }
                            val live = activity[conv.id]
                            val draft = drafts[conv.id].orEmpty()
                            val status = when {
                                draft.isNotBlank() -> stringResource(R.string.chat_draft)
                                live?.permissionRequest != null -> stringResource(R.string.conversations_waiting)
                                live?.processingState?.blocksInput == true -> stringResource(R.string.conversations_running)
                                else -> null
                            }
                            val preview = draft.takeIf { it.isNotBlank() } ?: live?.currentResponse?.takeIf { it.isNotBlank() } ?: previews[conv.id].orEmpty()
                            Text(buildAnnotatedString {
                                if (status != null) {
                                    withStyle(SpanStyle(color = MaterialTheme.colorScheme.primary, fontWeight = FontWeight.Medium)) { append(status) }
                                    append(" · ")
                                }
                                append(preview)
                            }, style = MaterialTheme.typography.bodyMedium.copy(fontSize = 14.sp, lineHeight = 22.sp),
                                color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }

                    }
                    expertChildren(conv)
                    if (conv.id != visible.lastOrNull()?.id) HorizontalDivider(
                        modifier = Modifier.padding(horizontal = nolaPageGutter()), thickness = 0.5.dp,
                        color = MaterialTheme.colorScheme.outlineVariant)
                }
            }
        }
    }
}
