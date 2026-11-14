package com.nola.ui.theme

import androidx.compose.ui.graphics.Color
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LightThemeContrastTest {
    @Test fun lightThemeTextColorPairsMeetNormalTextContrast() {
        val scheme = LightColors
        listOf(
            "primary button" to (scheme.primary to scheme.onPrimary),
            "primary text on canvas" to (scheme.onBackground to scheme.background),
            "primary text on surface" to (scheme.primary to scheme.surface),
            "secondary text on canvas" to (scheme.onSurfaceVariant to scheme.background),
            "secondary text on surface" to (scheme.onSurfaceVariant to scheme.surface),
            "error text on surface" to (scheme.error to scheme.surface),
            "tertiary button" to (scheme.tertiary to scheme.onTertiary),
        ).forEach { (label, pair) ->
            val a = luminance(pair.first)
            val b = luminance(pair.second)
            val ratio = (max(a, b) + 0.05) / (min(a, b) + 0.05)
            assertTrue("$label contrast $ratio is below 4.5:1", ratio >= 4.5)
        }
    }

    @Test fun darkThemeWarmInkSurfacesAndSemanticRolesMeetContrast() {
        val scheme = DarkColors
        assertEquals(Color(0xFF131312), scheme.background)
        assertEquals(Color(0xFF1B1B19), scheme.surface)
        assertEquals(Color(0xFF232320), scheme.surfaceVariant)
        assertEquals(Color(0xFFF2F0EB), scheme.onSurface)
        assertEquals(Color(0xFFA3A09A), scheme.onSurfaceVariant)
        assertEquals(Color(0xFF2E2D2A), scheme.outlineVariant)
        listOf(
            "dark primary button" to (scheme.primary to scheme.onPrimary),
            "dark primary text on canvas" to (scheme.onBackground to scheme.background),
            "dark primary text on surface" to (scheme.onSurface to scheme.surface),
            "dark secondary text on canvas" to (scheme.onSurfaceVariant to scheme.background),
            "dark secondary text on surface" to (scheme.onSurfaceVariant to scheme.surface),
            "dark secondary text on subtle" to (scheme.onSurfaceVariant to scheme.surfaceVariant),
            "dark error text on surface" to (scheme.error to scheme.surface),
            "dark tertiary button" to (scheme.tertiary to scheme.onTertiary),
        ).forEach { (label, pair) ->
            val a = luminance(pair.first)
            val b = luminance(pair.second)
            val ratio = (max(a, b) + 0.05) / (min(a, b) + 0.05)
            assertTrue("$label contrast $ratio is below 4.5:1", ratio >= 4.5)
        }
    }

    @Test fun mobileReadingAndControlsMeetContrastInBothThemes() {
        listOf(LightColors, DarkColors).forEach { scheme ->
            listOf(
                scheme.onBackground to scheme.background,
                scheme.onSurface to scheme.surface,
                scheme.onSurfaceVariant to scheme.background,
                scheme.onSurfaceVariant to scheme.surfaceVariant,
                scheme.onPrimaryContainer to scheme.primaryContainer,
                scheme.onPrimary to scheme.primary,
            ).forEach { (foreground, background) ->
                val a = luminance(foreground)
                val b = luminance(background)
                val ratio = (max(a, b) + 0.05) / (min(a, b) + 0.05)
                assertTrue("Mobile reading/control contrast $ratio is below 4.5:1", ratio >= 4.5)
            }
        }
    }

    private fun luminance(color: Color): Double {
        fun linear(component: Float): Double {
            val value = component.toDouble()
            return if (value <= 0.04045) value / 12.92 else ((value + 0.055) / 1.055).pow(2.4)
        }
        return linear(color.red) * 0.2126 + linear(color.green) * 0.7152 + linear(color.blue) * 0.0722
    }
}
