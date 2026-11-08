package com.nola.ui.theme

import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
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
    primary = Color(0xFF245B43),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFDFECE3),
    onPrimaryContainer = Color(0xFF29372E),
    secondary = Color(0xFF5B6B61),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFE4ECE7),
    onSecondaryContainer = Color(0xFF29372E),
    tertiary = Color(0xFF865400),
    onTertiary = Color.White,
    background = Color(0xFFF3F5F4),
    onBackground = Color(0xFF202923),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF202923),
    surfaceVariant = Color(0xFFE4ECE7),
    surfaceContainer = Color(0xFFE4ECE7),
    surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainerHigh = Color(0xFFE4ECE7),
    onSurfaceVariant = Color(0xFF5B6B61),
    error = Color(0xFFB42335),
    outline = Color(0xFF858585),
    outlineVariant = Color(0xFFE0E6E2)
)

internal val DarkColors = darkColorScheme(
    primary = Color(0xFFABD4BA),
    onPrimary = Color(0xFF14281D),
    primaryContainer = Color(0xFF293E32),
    onPrimaryContainer = Color(0xFFE8F0EB),
    secondary = Color(0xFFA5B6AB),
    onSecondary = Color(0xFF1B2420),
    secondaryContainer = Color(0xFF29372E),
    onSecondaryContainer = Color(0xFFE8F0EB),
    tertiary = Color(0xFFE9BD66),
    onTertiary = Color(0xFF302100),
    background = Color(0xFF111715),
    onBackground = Color(0xFFE8F0EB),
    surface = Color(0xFF1B2420),
    onSurface = Color(0xFFE8F0EB),
    surfaceVariant = Color(0xFF29372E),
    surfaceContainer = Color(0xFF29372E),
    surfaceContainerLow = Color(0xFF1B2420),
    surfaceContainerHigh = Color(0xFF29372E),
    onSurfaceVariant = Color(0xFFA5B6AB),
    error = Color(0xFFFF8A98),
    outline = Color(0xFF777777),
    outlineVariant = Color(0xFF303C35)
)

private val NolaTypography = Typography(
    headlineSmall = TextStyle(
        fontSize = 22.sp,
        lineHeight = 28.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    titleLarge = TextStyle(
        fontSize = 22.sp,
        lineHeight = 28.sp,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 0.sp
    ),
    titleMedium = TextStyle(
        fontSize = 17.sp,
        lineHeight = 23.sp,
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
        lineHeight = 24.sp,
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
        lineHeight = 16.sp,
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
        SideEffect {
            val activity = view.context.findComponentActivity() ?: return@SideEffect
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
            medium = NolaContinuousShape(14.dp), large = NolaContinuousShape(18.dp), extraLarge = NolaContinuousShape(24.dp)),
        content = content
    )
}

private tailrec fun Context.findComponentActivity(): ComponentActivity? = when (this) {
    is ComponentActivity -> this
    is ContextWrapper -> baseContext.findComponentActivity()
    else -> null
}
