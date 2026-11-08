package com.nola

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import com.nola.data.local.datastore.AppearancePreference
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.ui.Alignment
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.ui.navigation.NolaScaffold
import com.nola.ui.screen.onboarding.OnboardingScreen
import com.nola.ui.theme.NolaTheme
import dagger.hilt.android.AndroidEntryPoint
import javax.inject.Inject

@AndroidEntryPoint
class MainActivity : ComponentActivity() {

    @Inject
    lateinit var prefs: PreferenceDataStore

    @Inject
    lateinit var agentTurnService: com.nola.core.AndroidAgentTurnService

    override fun onStart() {
        super.onStart()
        agentTurnService.resumeEmbeddedChildren()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val appearance by prefs.appearance.collectAsState(initial = AppearancePreference.System)
            val darkTheme = when (appearance) {
                AppearancePreference.System -> isSystemInDarkTheme()
                AppearancePreference.Light -> false
                AppearancePreference.Dark -> true
            }
            NolaTheme(darkTheme = darkTheme) {
                val hasCompletedOnboarding by prefs.hasCompletedOnboarding.collectAsState(initial = null)

                if (hasCompletedOnboarding == null) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        CircularProgressIndicator()
                    }
                } else if (hasCompletedOnboarding == true) {
                    NolaScaffold(modifier = Modifier.fillMaxSize())
                } else {
                    OnboardingScreen()
                }
            }
        }
    }
}
