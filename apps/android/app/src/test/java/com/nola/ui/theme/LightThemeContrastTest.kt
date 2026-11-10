package com.nola.ui.theme

import androidx.compose.ui.graphics.Color
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import org.junit.Assert.assertTrue
import org.junit.Test

class LightThemeContrastTest {
    @Test fun lightThemeTextColorPairsMeetNormalTextContrast() {
        val scheme = LightColors
        listOf(
            "primary button" to (scheme.primary to scheme.onPrimary),
            "primary text on surface" to (scheme.primary to scheme.surface),
            "error text on surface" to (scheme.error to scheme.surface),
            "tertiary button" to (scheme.tertiary to scheme.onTertiary),
        ).forEach { (label, pair) ->
            val a = luminance(pair.first)
            val b = luminance(pair.second)
            val ratio = (max(a, b) + 0.05) / (min(a, b) + 0.05)
            assertTrue("$label contrast $ratio is below 4.5:1", ratio >= 4.5)
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
