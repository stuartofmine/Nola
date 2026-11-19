package com.nola.ui.screen.onboarding

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R

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
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background,
        contentColor = MaterialTheme.colorScheme.onBackground) {
        Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing),
            contentAlignment = Alignment.Center) {
            Column(
                modifier = Modifier.widthIn(max = 460.dp).fillMaxWidth()
                    .verticalScroll(rememberScrollState()).padding(horizontal = 28.dp, vertical = 24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(stringResource(R.string.onboarding_welcome_title),
                    modifier = Modifier.semantics { heading() }, style = MaterialTheme.typography.headlineMedium)
                Text(stringResource(R.string.onboarding_chat_model_hint),
                    color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge)
                if (uiState.completionFailed) {
                    Text(stringResource(R.string.onboarding_guide_save_error),
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                        color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                }
                Button(onClick = { onComplete(true) }, enabled = !uiState.isCompleting,
                    modifier = Modifier.padding(top = 12.dp).fillMaxWidth().heightIn(min = 48.dp)) {
                    Text(stringResource(if (uiState.isCompleting) R.string.onboarding_guide_action_saving
                        else R.string.onboarding_choose_model))
                }
                TextButton(onClick = { onComplete(false) }, enabled = !uiState.isCompleting,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.onboarding_guide_action_later))
                }
            }
        }
    }
}
