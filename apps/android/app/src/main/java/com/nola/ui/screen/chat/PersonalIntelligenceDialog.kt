package com.nola.ui.screen.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.nola.R
import com.nola.core.PersonalIntelligenceCore
import com.nola.ui.components.NolaMotion
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

private val IntelligenceAccent = Color(0xFF14988C)
private val IntelligenceBlue = Color(0xFF3C73DD)

@Composable
fun PersonalIntelligenceDialog(onDismiss: () -> Unit) {
    var snapshot by remember { mutableStateOf(PersonalIntelligenceCore.snapshot()) }
    var selectedFilter by remember { mutableStateOf(IntelligenceFilter.All) }
    var memoryLimit by remember(selectedFilter) { mutableStateOf(24) }
    var strategyLimit by remember { mutableStateOf(10) }
    var showClearConfirmation by remember { mutableStateOf(false) }
    var entered by remember { mutableStateOf(false) }
    val events = snapshot.array("events")
    val eventObjects = events.mapNotNull { runCatching { it.jsonObject }.getOrNull() }
    val memories = snapshot.array("memories").mapNotNull { runCatching { it.jsonObject }.getOrNull() }
    val activeMemories = memories.filter { it.string("status") == "active" }
    val filteredMemories = activeMemories.filter { memory ->
        selectedFilter.kind == null || memory.string("kind") == selectedFilter.kind
    }
    val strategies = snapshot.array("strategies").mapNotNull { runCatching { it.jsonObject }.getOrNull() }
    val sourceNames = eventObjects.mapNotNull { it.string("source") }.distinct().sorted()
    val averageConfidence = if (activeMemories.isEmpty()) 0 else {
        (activeMemories.sumOf { it.number("confidence") } / activeMemories.size * 100).toInt()
    }

    LaunchedEffect(Unit) { entered = true }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false)
    ) {
        Surface(
            modifier = Modifier
                .widthIn(max = 720.dp)
                .fillMaxWidth()
                .fillMaxHeight(0.94f)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            shape = RoundedCornerShape(30.dp),
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 6.dp,
            shadowElevation = 18.dp
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .background(
                        Brush.verticalGradient(
                            listOf(
                                IntelligenceAccent.copy(alpha = 0.075f),
                                MaterialTheme.colorScheme.surface,
                                MaterialTheme.colorScheme.surface,
                                IntelligenceBlue.copy(alpha = 0.035f)
                            )
                        )
                    )
            ) {
                IntelligenceTopBar(
                    title = stringResource(R.string.personal_intelligence_title),
                    refreshLabel = stringResource(R.string.personal_intelligence_refresh),
                    closeLabel = stringResource(R.string.personal_intelligence_close),
                    onRefresh = { snapshot = PersonalIntelligenceCore.snapshot() },
                    onDismiss = onDismiss
                )

                Column(
                    modifier = Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 18.dp)
                        .padding(bottom = 26.dp),
                    verticalArrangement = Arrangement.spacedBy(22.dp)
                ) {
                    AnimatedVisibility(
                        visible = entered,
                        enter = fadeIn(
                            tween(
                                durationMillis = NolaMotion.base,
                                easing = NolaMotion.emphasizedEasing,
                            )
                        ) + slideInVertically(
                            tween(
                                durationMillis = NolaMotion.slow,
                                easing = NolaMotion.emphasizedEasing,
                            )
                        ) { it / 16 }
                    ) {
                        IntelligenceHero(
                            observationCount = events.size,
                            memoryCount = activeMemories.size,
                            averageConfidence = averageConfidence,
                            sourceNames = sourceNames
                        )
                    }

                    IntelligenceSectionHeader(
                        stringResource(R.string.personal_intelligence_memories_title),
                        stringResource(R.string.personal_intelligence_memories_detail)
                    )

                    Row(
                        modifier = Modifier.horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        IntelligenceFilter.entries.forEach { filter ->
                            FilterChip(
                                selected = selectedFilter == filter,
                                onClick = { selectedFilter = filter },
                                label = { Text(filter.label()) }
                            )
                        }
                    }

                    Column(
                        modifier = Modifier.animateContentSize(
                            animationSpec = tween(
                                durationMillis = NolaMotion.base,
                                easing = NolaMotion.emphasizedEasing,
                            )
                        ),
                        verticalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        if (filteredMemories.isEmpty()) {
                            EmptyMemoryState(selectedFilter == IntelligenceFilter.All)
                        } else {
                            filteredMemories.take(memoryLimit).forEach { memory ->
                                IntelligenceMemoryCard(memory)
                            }
                            if (filteredMemories.size > memoryLimit) {
                                TextButton(onClick = { memoryLimit += 24 }) { Text(stringResource(R.string.intelligence_more)) }
                            }
                        }
                    }

                    if (strategies.isNotEmpty()) {
                        IntelligenceSectionHeader(
                            stringResource(R.string.personal_intelligence_strategies_title),
                            stringResource(R.string.personal_intelligence_strategies_detail)
                        )
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            strategies.sortedByDescending { it.number("score") }.take(strategyLimit).forEach { strategy ->
                                IntelligenceStrategyCard(strategy)
                            }
                            if (strategies.size > strategyLimit) {
                                TextButton(onClick = { strategyLimit += 10 }) { Text(stringResource(R.string.intelligence_more)) }
                            }
                        }
                    }

                    HowItWorksCard()

                    TextButton(
                        enabled = snapshot != null,
                        onClick = { showClearConfirmation = true },
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                    ) {
                        Icon(Icons.Filled.Delete, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(7.dp))
                        Text(stringResource(R.string.personal_intelligence_clear_action))
                    }
                }
            }
        }
    }

    if (showClearConfirmation) {
        AlertDialog(
            onDismissRequest = { showClearConfirmation = false },
            title = { Text(stringResource(R.string.personal_intelligence_clear_title)) },
            text = {
                Text(stringResource(R.string.personal_intelligence_clear_detail))
            },
            confirmButton = {
                TextButton(onClick = {
                    PersonalIntelligenceCore.clear()
                    snapshot = PersonalIntelligenceCore.snapshot()
                    showClearConfirmation = false
                }) { Text(stringResource(R.string.personal_intelligence_clear_confirm), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { showClearConfirmation = false }) { Text(stringResource(R.string.personal_intelligence_cancel)) }
            }
        )
    }
}

@Composable
private fun IntelligenceTopBar(
    title: String,
    refreshLabel: String,
    closeLabel: String,
    onRefresh: () -> Unit,
    onDismiss: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 20.dp, end = 10.dp, top = 10.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        Spacer(Modifier.weight(1f))
        IconButton(onClick = onRefresh) {
            Icon(Icons.Filled.Refresh, contentDescription = refreshLabel)
        }
        IconButton(onClick = onDismiss) {
            Icon(Icons.Filled.Close, contentDescription = closeLabel)
        }
    }
}

@Composable
private fun IntelligenceHero(
    observationCount: Int,
    memoryCount: Int,
    averageConfidence: Int,
    sourceNames: List<String>
) {
    Surface(
        shape = RoundedCornerShape(26.dp),
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0.86f),
        tonalElevation = 2.dp
    ) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(verticalAlignment = Alignment.Top) {
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .clip(CircleShape)
                        .background(Brush.linearGradient(listOf(IntelligenceAccent, IntelligenceBlue))),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(Icons.Filled.AutoAwesome, contentDescription = null, tint = Color.White)
                }
                Spacer(Modifier.width(13.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.size(7.dp).clip(CircleShape).background(IntelligenceAccent))
                        Spacer(Modifier.width(7.dp))
                        Text(
                            stringResource(R.string.personal_intelligence_learning_status),
                            style = MaterialTheme.typography.labelMedium,
                            color = IntelligenceAccent,
                            fontWeight = FontWeight.SemiBold
                        )
                    }
                    Text(
                        stringResource(R.string.personal_intelligence_hero_title),
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold
                    )
                    Text(
                        stringResource(R.string.personal_intelligence_hero_detail),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                IntelligenceMetric(stringResource(R.string.personal_intelligence_metric_observed), observationCount.toString(), Modifier.weight(1f))
                IntelligenceMetric(stringResource(R.string.personal_intelligence_metric_memories), memoryCount.toString(), Modifier.weight(1f))
                IntelligenceMetric(stringResource(R.string.personal_intelligence_metric_confidence), "$averageConfidence%", Modifier.weight(1f))
            }

            if (sourceNames.isNotEmpty()) {
                Column(verticalArrangement = Arrangement.spacedBy(7.dp)) {
                    Text(
                        stringResource(R.string.personal_intelligence_sources_title),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Row(
                        modifier = Modifier.horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(7.dp)
                    ) {
                        sourceNames.forEach { source ->
                            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.055f)) {
                                Text(
                                    sourceLabel(source),
                                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                                    style = MaterialTheme.typography.labelSmall
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun IntelligenceMetric(label: String, value: String, modifier: Modifier = Modifier) {
    Surface(modifier = modifier, shape = RoundedCornerShape(15.dp), color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.045f)) {
        Column(Modifier.padding(11.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(value, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
        }
    }
}

@Composable
private fun IntelligenceSectionHeader(title: String, subtitle: String) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
        Text(subtitle, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun EmptyMemoryState(isAll: Boolean) {
    Surface(shape = RoundedCornerShape(22.dp), color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.035f)) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(9.dp)
        ) {
            Icon(Icons.Filled.AutoAwesome, contentDescription = null, tint = IntelligenceAccent)
            Text(
                stringResource(if (isAll) R.string.personal_intelligence_empty_all else R.string.personal_intelligence_empty_filter),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold
            )
            Text(
                stringResource(R.string.personal_intelligence_empty_detail),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun IntelligenceMemoryCard(memory: JsonObject) {
    val kind = memory.string("kind")
    val confidence = memory.number("confidence").coerceIn(0.0, 1.0)
    val evidenceCount = memory.array("source_event_ids").size
    val color = memoryColor(kind)

    Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.48f)) {
        Column(Modifier.padding(15.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Surface(shape = CircleShape, color = color.copy(alpha = 0.11f)) {
                    Text(
                        memoryKind(kind),
                        modifier = Modifier.padding(horizontal = 9.dp, vertical = 5.dp),
                        style = MaterialTheme.typography.labelSmall,
                        color = color,
                        fontWeight = FontWeight.SemiBold
                    )
                }
                Spacer(Modifier.weight(1f))
                Text(
                    stringResource(R.string.personal_intelligence_evidence_count, evidenceCount),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Text(memory.string("statement").orEmpty(), style = MaterialTheme.typography.bodyMedium)
            Row(verticalAlignment = Alignment.CenterVertically) {
                LinearProgressIndicator(
                    progress = { confidence.toFloat() },
                    modifier = Modifier.weight(1f).height(4.dp).clip(CircleShape),
                    color = color,
                    trackColor = color.copy(alpha = 0.12f)
                )
                Spacer(Modifier.width(10.dp))
                Text(
                    "${(confidence * 100).toInt()}%",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
private fun IntelligenceStrategyCard(strategy: JsonObject) {
    val score = strategy.number("score").coerceIn(0.0, 1.0)
    val model = strategy.string("model").takeUnless { it.isNullOrBlank() }
        ?: strategy.string("route").orEmpty()
    Surface(shape = RoundedCornerShape(19.dp), color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.04f)) {
        Row(Modifier.fillMaxWidth().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier.size(38.dp).clip(CircleShape).background(IntelligenceBlue.copy(alpha = 0.11f)),
                contentAlignment = Alignment.Center
            ) {
                Icon(Icons.Filled.Storage, contentDescription = null, tint = IntelligenceBlue, modifier = Modifier.size(19.dp))
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(model, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    "${strategy.string("task_type").orEmpty()} · ${stringResource(R.string.personal_intelligence_outcomes_count, strategy.number("attempts").toInt())}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text("${(score * 100).toInt()}", color = IntelligenceBlue, fontWeight = FontWeight.Bold)
                Text(stringResource(R.string.personal_intelligence_fit), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun HowItWorksCard() {
    Surface(shape = RoundedCornerShape(22.dp), color = IntelligenceAccent.copy(alpha = 0.065f)) {
        Column(Modifier.padding(17.dp), verticalArrangement = Arrangement.spacedBy(13.dp)) {
            IntelligenceSectionHeader(
                stringResource(R.string.personal_intelligence_how_title),
                stringResource(R.string.personal_intelligence_how_detail)
            )
            ExplanationRow(stringResource(R.string.personal_intelligence_observe_title), stringResource(R.string.personal_intelligence_observe_detail))
            ExplanationRow(stringResource(R.string.personal_intelligence_evidence_title), stringResource(R.string.personal_intelligence_evidence_detail))
            ExplanationRow(stringResource(R.string.personal_intelligence_outcomes_title), stringResource(R.string.personal_intelligence_outcomes_detail))
        }
    }
}

@Composable
private fun ExplanationRow(title: String, detail: String) {
    Row(verticalAlignment = Alignment.Top) {
        Icon(Icons.Filled.CheckCircle, contentDescription = null, tint = IntelligenceAccent, modifier = Modifier.size(19.dp))
        Spacer(Modifier.width(10.dp))
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold)
            Text(detail, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private enum class IntelligenceFilter(val kind: String?, val labelResource: Int) {
    All(null, R.string.personal_intelligence_filter_all),
    Facts("semantic", R.string.personal_intelligence_filter_facts),
    Preferences("procedural", R.string.personal_intelligence_filter_preferences),
    Episodes("episodic", R.string.personal_intelligence_filter_episodes);

    @Composable
    fun label(): String = stringResource(labelResource)
}

private fun JsonObject?.array(key: String): JsonArray =
    runCatching { this?.get(key)?.jsonArray }.getOrNull() ?: JsonArray(emptyList())

private fun JsonObject.string(key: String): String? =
    runCatching { get(key)?.jsonPrimitive?.contentOrNull }.getOrNull()

private fun JsonObject.number(key: String): Double =
    runCatching { get(key)?.jsonPrimitive?.doubleOrNull }.getOrNull() ?: 0.0

@Composable
private fun memoryKind(raw: String?): String = when (raw) {
    "procedural" -> stringResource(R.string.personal_intelligence_kind_preference)
    "episodic" -> stringResource(R.string.personal_intelligence_kind_episode)
    else -> stringResource(R.string.personal_intelligence_kind_fact)
}

private fun memoryColor(raw: String?): Color = when (raw) {
    "procedural" -> Color(0xFFD77A17)
    "episodic" -> IntelligenceBlue
    else -> IntelligenceAccent
}

@Composable
private fun sourceLabel(source: String): String = when (source) {
    "conversation" -> stringResource(R.string.personal_intelligence_source_conversation)
    "health" -> stringResource(R.string.personal_intelligence_source_health)
    "attachment" -> stringResource(R.string.personal_intelligence_source_attachment)
    else -> source.replace('_', ' ').replaceFirstChar { it.uppercase() }
}
