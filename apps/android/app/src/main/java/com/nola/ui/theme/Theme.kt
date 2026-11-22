package com.nola.ui.theme

import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat

internal val LightColors = lightColorScheme(
    primary = Color(0xFF18181B),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFEBEBEF),
    onPrimaryContainer = Color(0xFF111113),
    secondary = Color(0xFF52525B),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFF2F2F4),
    onSecondaryContainer = Color(0xFF111113),
    tertiary = Color(0xFF925605),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFF6E8C8),
    onTertiaryContainer = Color(0xFF3A2500),
    background = Color(0xFFFAFAFA),
    onBackground = Color(0xFF111113),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF111113),
    surfaceDim = Color(0xFFEBEBEF),
    surfaceBright = Color(0xFFFAFAFA),
    surfaceVariant = Color(0xFFF2F2F4),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainer = Color(0xFFF2F2F4),
    surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainerHigh = Color(0xFFFFFFFF),
    surfaceContainerHighest = Color(0xFFE4E4E7),
    onSurfaceVariant = Color(0xFF52525B),
    inverseSurface = Color(0xFF18181B),
    inverseOnSurface = Color(0xFFFAFAFA),
    surfaceTint = Color(0xFF18181B),
    error = Color(0xFFC81E1E),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFF8E1DF),
    onErrorContainer = Color(0xFF5C1914),
    outline = Color(0xFF71717A),
    outlineVariant = Color(0xFFE4E4E7)
)

internal val DarkColors = darkColorScheme(
    primary = Color(0xFFF5F5F7),
    onPrimary = Color(0xFF0B0B0C),
    primaryContainer = Color(0xFF202024),
    onPrimaryContainer = Color(0xFFF5F5F7),
    secondary = Color(0xFFA1A1AA),
    onSecondary = Color(0xFF0B0B0C),
    secondaryContainer = Color(0xFF1a1a1d),
    onSecondaryContainer = Color(0xFFF5F5F7),
    tertiary = Color(0xFFF0B85A),
    onTertiary = Color(0xFF302100),
    tertiaryContainer = Color(0xFF473413),
    onTertiaryContainer = Color(0xFFF7E4B6),
    background = Color(0xFF0b0b0c),
    onBackground = Color(0xFFF5F5F7),
    surface = Color(0xFF131315),
    onSurface = Color(0xFFF5F5F7),
    surfaceDim = Color(0xFF0b0b0c),
    surfaceBright = Color(0xFF2C2C30),
    surfaceVariant = Color(0xFF1a1a1d),
    surfaceContainerLowest = Color(0xFF0b0b0c),
    surfaceContainer = Color(0xFF1a1a1d),
    surfaceContainerLow = Color(0xFF131315),
    surfaceContainerHigh = Color(0xFF242428),
    surfaceContainerHighest = Color(0xFF26262a),
    onSurfaceVariant = Color(0xFFA1A1AA),
    inverseSurface = Color(0xFFF2F2F4),
    inverseOnSurface = Color(0xFF111113),
    surfaceTint = Color(0xFFF5F5F7),
    error = Color(0xFFF28080),
    onError = Color(0xFF47110E),
    errorContainer = Color(0xFF542522),
    onErrorContainer = Color(0xFFFBDCD9),
    outline = Color(0xFF7a7a82),
    outlineVariant = Color(0xFF26262a)
)

private val NolaTypography = Typography(
    headlineSmall = TextStyle(
        fontSize = 28.sp,
        lineHeight = 36.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    titleLarge = TextStyle(
        fontSize = 20.sp,
        lineHeight = 28.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    titleMedium = TextStyle(
        fontSize = 16.sp,
        lineHeight = 24.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    titleSmall = TextStyle(
        fontSize = 15.sp,
        lineHeight = 21.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    bodyLarge = TextStyle(
        fontSize = 17.sp,
        lineHeight = 28.sp,
        fontWeight = FontWeight.Normal,
        letterSpacing = 0.sp
    ),
    bodyMedium = TextStyle(
        fontSize = 16.sp,
        lineHeight = 26.sp,
        fontWeight = FontWeight.Normal,
        letterSpacing = 0.sp
    ),
    bodySmall = TextStyle(
        fontSize = 13.sp,
        lineHeight = 18.sp,
        fontWeight = FontWeight.Normal,
        letterSpacing = 0.sp
    ),
    labelLarge = TextStyle(
        fontSize = 15.sp,
        lineHeight = 20.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    labelMedium = TextStyle(
        fontSize = 13.sp,
        lineHeight = 18.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    labelSmall = TextStyle(
        fontSize = 12.sp,
        lineHeight = 18.sp,
        fontWeight = FontWeight.Medium,
        letterSpacing = 0.sp
    )
)

@Composable
fun NolaTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColors
        else -> LightColors
    }

    val view = LocalView.current
    if (!view.isInEditMode) {
        LaunchedEffect(darkTheme) {
            val activity = view.context.findComponentActivity() ?: return@LaunchedEffect
            val statusBarScrim = colorScheme.background.toArgb()
            val navigationBarScrim = colorScheme.surface.toArgb()
            activity.enableEdgeToEdge(
                statusBarStyle = SystemBarStyle.auto(
                    lightScrim = statusBarScrim,
                    darkScrim = statusBarScrim,
                    detectDarkMode = { darkTheme }
                ),
                navigationBarStyle = SystemBarStyle.auto(
                    lightScrim = navigationBarScrim,
                    darkScrim = navigationBarScrim,
                    detectDarkMode = { darkTheme }
                )
            )
            val window = activity.window
            // Transparent system bars can expose the light activity window
            // after recreation. Keep their backing surface in the current theme.
            window.setBackgroundDrawable(android.graphics.drawable.ColorDrawable(statusBarScrim))
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = !darkTheme
            WindowCompat.getInsetsController(window, view).isAppearanceLightNavigationBars = !darkTheme
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = NolaTypography,
        shapes = Shapes(extraSmall = NolaContinuousShape(6.dp), small = NolaContinuousShape(10.dp),
            medium = NolaContinuousShape(14.dp), large = NolaContinuousShape(18.dp), extraLarge = NolaContinuousShape(24.dp))
    ) {
        CompositionLocalProvider(LocalContentColor provides colorScheme.onBackground) {
            content()
        }
    }
}

private tailrec fun Context.findComponentActivity(): ComponentActivity? = when (this) {
    is ComponentActivity -> this
    is ContextWrapper -> baseContext.findComponentActivity()
    else -> null
}
