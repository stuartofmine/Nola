package com.nola.ui.screen.onboarding

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.CalendarMonth
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.nola.ui.components.NolaMotion
import androidx.hilt.navigation.compose.hiltViewModel
import com.nola.R

private const val GuideStepCount = 4

@Composable
fun OnboardingScreen(viewModel: OnboardingViewModel = hiltViewModel()) {
    val uiState by viewModel.uiState.collectAsState()
    var step by rememberSaveable { mutableIntStateOf(0) }
    BackHandler(enabled = step > 0 && !uiState.isCompleting) { step-- }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .padding(horizontal = 20.dp),
        contentAlignment = Alignment.TopCenter
    ) {
        Column(
            modifier = Modifier
                .fillMaxHeight()
                .widthIn(max = 720.dp)
                .fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            GuideHeader(step)
            androidx.compose.material3.TextButton(
                onClick = { viewModel.completeOnboarding() },
                enabled = !uiState.isCompleting,
            ) { Text(stringResource(R.string.onboarding_guide_action_later)) }

            AnimatedContent(
                targetState = step,
                transitionSpec = {
                    fadeIn(
                        tween(
                            durationMillis = NolaMotion.base,
                            easing = NolaMotion.emphasizedEasing,
                        )
                    ) togetherWith fadeOut(tween(0))
                },
                label = "nola-guide-step",
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
            ) { current ->
                when (current) {
                    0 -> ValuePage()
                    1 -> RuntimePage()
                    2 -> SourcesPage()
                    else -> ReadyPage()
                }
            }

            AnimatedVisibility(
                visible = uiState.completionFailed,
                enter = fadeIn(tween(NolaMotion.base)) + expandVertically(
                    tween(NolaMotion.base),
                    expandFrom = Alignment.Top,
                ),
                exit = fadeOut(tween(NolaMotion.fast)) + shrinkVertically(
                    tween(NolaMotion.fast),
                    shrinkTowards = Alignment.Top,
                )
            ) {
                Text(
                    stringResource(R.string.onboarding_guide_save_error),
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 8.dp, bottom = 4.dp),
                    textAlign = TextAlign.Center
                )
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 10.dp, bottom = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                if (step > 0) {
                    IconButton(
                        onClick = { step-- },
                        enabled = !uiState.isCompleting,
                        modifier = Modifier
                            .size(50.dp)
                            .background(
                                MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.62f),
                                RoundedCornerShape(15.dp)
                            )
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            stringResource(R.string.onboarding_guide_back)
                        )
                    }
                } else {
                    Spacer(Modifier.size(50.dp))
                }
                Button(
                    onClick = {
                        if (step < GuideStepCount - 1) {
                            step++
                        } else {
                            viewModel.completeOnboarding()
                        }
                    },
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = 50.dp),
                    shape = RoundedCornerShape(15.dp),
                    enabled = !uiState.isCompleting
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Box(
                            modifier = Modifier.size(20.dp),
                            contentAlignment = Alignment.Center
                        ) {
                            if (uiState.isCompleting) {
                                CircularProgressIndicator(
                                    modifier = Modifier.size(18.dp),
                                    strokeWidth = 2.dp,
                                    color = MaterialTheme.colorScheme.onPrimary
                                )
                            }
                        }
                        Text(
                            text = if (uiState.isCompleting) {
                                stringResource(R.string.onboarding_guide_action_saving)
                            } else if (step == GuideStepCount - 1) {
                                stringResource(R.string.onboarding_guide_action_start)
                            } else {
                                stringResource(R.string.onboarding_guide_action_next)
                            },
                            modifier = Modifier.weight(1f),
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center
                        )
                        Box(
                            modifier = Modifier.size(20.dp),
                            contentAlignment = Alignment.Center
                        ) {
                            if (!uiState.isCompleting) {
                                Icon(
                                    if (step == GuideStepCount - 1) Icons.Filled.Check else Icons.AutoMirrored.Filled.ArrowForward,
                                    contentDescription = null
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
private fun GuideHeader(step: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 22.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.primary.copy(alpha = 0.11f)
        ) {
            Icon(
                Icons.Filled.Storage,
                contentDescription = null,
                modifier = Modifier.padding(10.dp),
                tint = MaterialTheme.colorScheme.primary
            )
        }
        Column(
            modifier = Modifier
                .padding(start = 11.dp)
                .weight(1f)
        ) {
            Text("Nola", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            Text(
                stringResource(R.string.onboarding_core_brand_subtitle),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Text(
            stringResource(R.string.onboarding_guide_step_format, step + 1, GuideStepCount),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .semantics {
                progressBarRangeInfo = ProgressBarRangeInfo(
                    current = (step + 1).toFloat(),
                    range = 1f..GuideStepCount.toFloat(),
                    steps = GuideStepCount - 2
                )
            },
        horizontalArrangement = Arrangement.spacedBy(7.dp)
    ) {
        repeat(GuideStepCount) { index ->
            Box(
                modifier = Modifier
                    .weight(1f)
                    .height(4.dp)
                    .background(
                        if (index <= step) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            MaterialTheme.colorScheme.outline.copy(alpha = 0.16f)
                        },
                        RoundedCornerShape(50)
                    )
            )
        }
    }
}

@Composable
private fun ValuePage() {
    GuidePage(
        eyebrow = stringResource(R.string.onboarding_core_loop_eyebrow),
        title = stringResource(R.string.onboarding_core_loop_title),
        body = stringResource(R.string.onboarding_core_loop_body)
    ) {
        Surface(
            shape = RoundedCornerShape(22.dp),
            color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f)
        ) {
            Column(modifier = Modifier.padding(horizontal = 18.dp, vertical = 17.dp)) {
                FlowRow(
                    Icons.Filled.Storage,
                    stringResource(R.string.onboarding_core_source_title),
                    stringResource(R.string.onboarding_core_source_detail),
                    MaterialTheme.colorScheme.primary
                )
                FlowConnector()
                FlowRow(
                    Icons.AutoMirrored.Filled.Chat,
                    stringResource(R.string.onboarding_core_answer_title),
                    stringResource(R.string.onboarding_core_answer_detail),
                    MaterialTheme.colorScheme.secondary
                )
                FlowConnector()
                FlowRow(
                    Icons.Filled.Refresh,
                    stringResource(R.string.onboarding_core_learn_title),
                    stringResource(R.string.onboarding_core_learn_detail),
                    MaterialTheme.colorScheme.tertiary
                )
            }
        }
        GuideNote(Icons.Filled.Favorite, stringResource(R.string.onboarding_core_local_note))
    }
}

@Composable
private fun RuntimePage() {
    GuidePage(
        eyebrow = stringResource(R.string.onboarding_guide_runtime_eyebrow),
        title = stringResource(R.string.onboarding_guide_runtime_title),
        body = stringResource(R.string.onboarding_guide_runtime_body)
    ) {
        TaskRow(
            icon = Icons.Filled.Refresh,
            title = stringResource(R.string.onboarding_guide_runtime_auto),
            detail = stringResource(R.string.onboarding_guide_runtime_auto_detail)
        )
        TaskRow(
            icon = Icons.Filled.Public,
            title = stringResource(R.string.onboarding_guide_runtime_host),
            detail = stringResource(R.string.onboarding_guide_runtime_host_detail)
        )
        GuideNote(Icons.Filled.Check, stringResource(R.string.onboarding_guide_runtime_note))
    }
}

@Composable
private fun SourcesPage() {
    GuidePage(
        eyebrow = stringResource(R.string.onboarding_core_sources_eyebrow),
        title = stringResource(R.string.onboarding_guide_sources_title),
        body = stringResource(R.string.onboarding_guide_sources_body)
    ) {
        TaskRow(
            icon = Icons.Filled.Description,
            title = stringResource(R.string.onboarding_guide_source_documents),
            detail = stringResource(R.string.onboarding_guide_source_documents_detail)
        )
        TaskRow(
            icon = Icons.Filled.CalendarMonth,
            title = stringResource(R.string.onboarding_guide_source_calendar),
            detail = stringResource(R.string.onboarding_guide_source_calendar_detail)
        )
        TaskRow(
            icon = Icons.Filled.Favorite,
            title = stringResource(R.string.onboarding_guide_source_health),
            detail = stringResource(R.string.onboarding_core_health_detail)
        )
        GuideNote(
            Icons.Filled.Check,
            stringResource(R.string.onboarding_guide_sources_permission_note)
        )
    }
}

@Composable
private fun ReadyPage() {
    GuidePage(
        eyebrow = stringResource(R.string.onboarding_guide_ready_eyebrow),
        title = stringResource(R.string.onboarding_guide_ready_title),
        body = stringResource(R.string.onboarding_guide_ready_body)
    ) {
        TaskRow(
            Icons.Filled.CalendarMonth,
            stringResource(R.string.onboarding_guide_task_day),
            stringResource(R.string.onboarding_guide_task_day_detail)
        )
        TaskRow(
            Icons.Filled.Description,
            stringResource(R.string.onboarding_guide_task_document),
            stringResource(R.string.onboarding_guide_task_document_detail)
        )
        TaskRow(
            Icons.Filled.Public,
            stringResource(R.string.onboarding_guide_task_research),
            stringResource(R.string.onboarding_guide_task_research_detail)
        )
        GuideNote(Icons.Filled.Refresh, stringResource(R.string.onboarding_guide_ready_memory_note))
    }
}

@Composable
private fun GuidePage(
    eyebrow: String,
    title: String,
    body: String,
    content: @Composable ColumnScope.() -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(vertical = 28.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        Text(
            eyebrow,
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.primary
        )
        Text(
            title,
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold
        )
        Text(
            body,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Spacer(Modifier.height(2.dp))
        content()
    }
}

@Composable
private fun FlowRow(icon: ImageVector, title: String, detail: String, tint: Color) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Surface(shape = RoundedCornerShape(11.dp), color = tint.copy(alpha = 0.11f)) {
            Icon(icon, null, tint = tint, modifier = Modifier.padding(10.dp))
        }
        Column(
            modifier = Modifier
                .padding(start = 13.dp)
                .weight(1f)
        ) {
            Text(title, fontWeight = FontWeight.SemiBold)
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun FlowConnector() {
    Box(
        Modifier
            .padding(start = 19.dp)
            .size(width = 1.dp, height = 14.dp)
            .background(MaterialTheme.colorScheme.outline.copy(alpha = 0.25f))
    )
}

@Composable
private fun TaskRow(icon: ImageVector, title: String, detail: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f), RoundedCornerShape(18.dp))
            .padding(horizontal = 15.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Surface(shape = RoundedCornerShape(11.dp), color = MaterialTheme.colorScheme.primary.copy(alpha = 0.1f)) {
            Icon(icon, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(10.dp))
        }
        Column(
            Modifier
                .padding(start = 13.dp)
                .weight(1f)
        ) {
            Text(title, fontWeight = FontWeight.SemiBold)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun GuideNote(icon: ImageVector, text: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(
                MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.28f),
                RoundedCornerShape(15.dp)
            )
            .padding(horizontal = 13.dp, vertical = 11.dp),
        verticalAlignment = Alignment.Top
    ) {
        Icon(
            icon,
            null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(18.dp)
        )
        Text(
            text,
            modifier = Modifier.padding(start = 9.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Start
        )
    }
}
