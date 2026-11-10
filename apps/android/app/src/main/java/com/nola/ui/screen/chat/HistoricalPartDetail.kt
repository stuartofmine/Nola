package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import com.nola.R
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.domain.projection.ConversationProjectionV1
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext

@EntryPoint
@InstallIn(SingletonComponent::class)
interface HistoricalPartRuntimeEntryPoint {
    fun applicationRuntime(): AndroidApplicationRuntime
}

/** Detail state is deliberately independent of the timeline reducer and its sequence. */
@Composable
fun HistoricalPartActions(parts: List<ConversationProjectionV1.Part.Historical>) {
    var selected by remember(parts) { mutableStateOf<ConversationProjectionV1.HistoricalProjection?>(null) }
    parts.forEach { part ->
        TextButton(onClick = { selected = part.projection }) {
            Text(stringResource(R.string.historical_details) + " · " + part.projection.canonicalKind)
        }
    }
    selected?.let { request ->
        val context = LocalContext.current.applicationContext
        val runtime = remember(context) {
            EntryPointAccessors.fromApplication(context, HistoricalPartRuntimeEntryPoint::class.java).applicationRuntime()
        }
        key(request) {
            HistoricalPartDetail(request, { selected = null }) {
                withContext(Dispatchers.IO) { runtime.readHistoricalPart(request.owner, request.canonicalRevision) }
            }
        }
    }
}

internal suspend fun loadHistoricalPartDetail(
    request: ConversationProjectionV1.HistoricalProjection,
    unavailable: String,
    reader: suspend () -> ConversationProjectionV1.HistoricalProjection?,
): ConversationProjectionV1.HistoricalProjection {
    val response = reader()
    coroutineContext.ensureActive()
    require(response != null && response.owner == request.owner &&
        response.canonicalRevision == request.canonicalRevision && response.canonicalKind == request.canonicalKind) { unavailable }
    return response
}

@Composable
internal fun HistoricalPartDetail(
    request: ConversationProjectionV1.HistoricalProjection,
    onDismiss: () -> Unit,
    reader: suspend () -> ConversationProjectionV1.HistoricalProjection?,
) {
    var attempt by remember { mutableIntStateOf(0) }
    var result by remember { mutableStateOf<ConversationProjectionV1.HistoricalProjection?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }
    val unavailable = stringResource(R.string.historical_unavailable)
    LaunchedEffect(request, attempt) {
        loading = true
        failure = null
        result = null
        try {
            result = loadHistoricalPartDetail(request, unavailable, reader)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            failure = error.message ?: unavailable
        } finally {
            if (coroutineContext[kotlinx.coroutines.Job]?.isActive == true) loading = false
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.historical_details)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                if (loading) CircularProgressIndicator()
                failure?.let { Text(it) }
                result?.let { projection ->
                    Text(projection.statusLabel)
                    if (projection.status == ConversationProjectionV1.HistoricalStatus.NotLoaded ||
                        projection.status == ConversationProjectionV1.HistoricalStatus.Deferred) {
                        Text(stringResource(R.string.historical_limited))
                    }
                    projection.fields.toSortedMap().forEach { (name, field) ->
                        Text(name)
                        when (field) {
                            is ConversationProjectionV1.HistoricalField.Complete -> Text(field.value.toString())
                            is ConversationProjectionV1.HistoricalField.Deferred -> Text(stringResource(R.string.historical_field_deferred))
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(android.R.string.ok)) } },
        dismissButton = {
            TextButton(onClick = { attempt++ }, enabled = !loading) { Text(stringResource(R.string.historical_reload)) }
        },
    )
}
