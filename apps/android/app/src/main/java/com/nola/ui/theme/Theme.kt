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
    primary = Color(0xFF181817),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFEAE8E2),
    onPrimaryContainer = Color(0xFF181817),
    secondary = Color(0xFF5C5B57),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFF1F0EC),
    onSecondaryContainer = Color(0xFF181817),
    tertiary = Color(0xFF865400),
    onTertiary = Color(0xFFFFFFFF),
    background = Color(0xFFF7F6F3),
    onBackground = Color(0xFF181817),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF181817),
    surfaceVariant = Color(0xFFF1F0EC),
    surfaceContainer = Color(0xFFF1F0EC),
    surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainerHigh = Color(0xFFEAE8E2),
    onSurfaceVariant = Color(0xFF5C5B57),
    error = Color(0xFFAD3D34),
    outline = Color(0xFF7A7872),
    outlineVariant = Color(0xFFE5E3DD)
)

internal val DarkColors = darkColorScheme(
    primary = Color(0xFFF2F0EB),
    onPrimary = Color(0xFF131312),
    primaryContainer = Color(0xFF292825),
    onPrimaryContainer = Color(0xFFF2F0EB),
    secondary = Color(0xFFA3A09A),
    onSecondary = Color(0xFF131312),
    secondaryContainer = Color(0xFF232320),
    onSecondaryContainer = Color(0xFFF2F0EB),
    tertiary = Color(0xFFE9BD66),
    onTertiary = Color(0xFF302100),
    background = Color(0xFF131312),
    onBackground = Color(0xFFF2F0EB),
    surface = Color(0xFF1B1B19),
    onSurface = Color(0xFFF2F0EB),
    surfaceVariant = Color(0xFF232320),
    surfaceContainer = Color(0xFF232320),
    surfaceContainerLow = Color(0xFF1B1B19),
    surfaceContainerHigh = Color(0xFF292825),
    onSurfaceVariant = Color(0xFFA3A09A),
    error = Color(0xFFEFAAA2),
    outline = Color(0xFF827F79),
    outlineVariant = Color(0xFF2E2D2A)
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
