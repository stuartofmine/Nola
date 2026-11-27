package com.nola.ui.screen.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaIconBadge
import com.nola.ui.components.NolaPrimaryButton
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.NolaSpacing
import com.nola.ui.theme.NolaContinuousShape

@Composable
fun OnboardingScreen(
    onChooseDestination: (openModels: Boolean) -> Unit,
    viewModel: OnboardingViewModel = hiltViewModel(),
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    OnboardingContent(uiState) { openModels ->
        if (viewModel.completeOnboarding()) onChooseDestination(openModels)
    }
}

@Composable
internal fun OnboardingContent(uiState: OnboardingUiState, onComplete: (openModels: Boolean) -> Unit) {
    Surface(
        Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
        contentColor = MaterialTheme.colorScheme.onBackground,
        tonalElevation = 0.dp,
    ) {
        Box(
            Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 460.dp)
                    .fillMaxSize()
                    .padding(horizontal = 24.dp, vertical = 16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Box(
                            modifier = Modifier
                                .size(64.dp)
                                .clip(NolaContinuousShape(NolaRadius.xl))
                                .background(MaterialTheme.colorScheme.primary),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                text = "n",
                                style = MaterialTheme.typography.headlineLarge.copy(
                                    fontSize = 36.sp,
                                    fontWeight = FontWeight.Bold,
                                ),
                                color = MaterialTheme.colorScheme.onPrimary,
                            )
                        }
                        Column(
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            Text(
                                stringResource(R.string.onboarding_welcome_title),
                                modifier = Modifier.semantics { heading() },
                                style = MaterialTheme.typography.headlineMedium,
                                textAlign = TextAlign.Center,
                            )
                            Text(
                                stringResource(R.string.onboarding_chat_model_hint),
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                style = MaterialTheme.typography.bodyMedium,
                                textAlign = TextAlign.Center,
                            )
                        }
                        NolaCard {
                            OnboardingCapabilityRow(
                                icon = Icons.Filled.Memory,
                                title = stringResource(R.string.onboarding_cap_local_title),
                                description = stringResource(R.string.onboarding_cap_local_desc),
                            )
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 0.5.dp)
                            OnboardingCapabilityRow(
                                icon = Icons.Filled.Cloud,
                                title = stringResource(R.string.onboarding_cap_cloud_title),
                                description = stringResource(R.string.onboarding_cap_cloud_desc),
                            )
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant, thickness = 0.5.dp)
                            OnboardingCapabilityRow(
                                icon = Icons.Filled.Computer,
                                title = stringResource(R.string.onboarding_cap_host_title),
                                description = stringResource(R.string.onboarding_cap_host_desc),
                            )
                        }
                    }
                }
                if (uiState.completionFailed) {
                    Text(
                        stringResource(R.string.onboarding_guide_save_error),
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    NolaPrimaryButton(
                        text = stringResource(
                            if (uiState.isCompleting) R.string.onboarding_guide_action_saving
                            else R.string.onboarding_choose_model
                        ),
                        onClick = { onComplete(true) },
                        enabled = !uiState.isCompleting,
                    )
                    TextButton(
                        onClick = { onComplete(false) },
                        enabled = !uiState.isCompleting,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    ) {
                        Text(stringResource(R.string.onboarding_guide_action_later))
                    }
                }
            }
        }
    }
}

@Composable
private fun OnboardingCapabilityRow(
    icon: ImageVector,
    title: String,
    description: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NolaSpacing.md),
        verticalAlignment = Alignment.Top,
    ) {
        NolaIconBadge(icon = icon, size = 34.dp)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
