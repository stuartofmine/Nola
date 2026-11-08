package com.nola.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertTrue
import org.junit.Test

class InkMountainContrastTest {
    @Test fun readableSemanticPairsInBothAppearances() {
        listOf(LightColors, DarkColors).forEach { colors ->
            listOf(colors.onBackground to colors.background, colors.onSurface to colors.surface,
                colors.onSurfaceVariant to colors.surfaceContainer, colors.onPrimary to colors.primary,
                colors.primary to colors.surface, colors.error to colors.surface).forEach { (text, surface) ->
                assertTrue("Semantic text contrast must reach 4.5:1", contrast(text, surface) >= 4.5f)
            }
        }
    }
    private fun contrast(first: Color, second: Color): Float {
        val a = first.luminance(); val b = second.luminance()
        return (maxOf(a, b) + 0.05f) / (minOf(a, b) + 0.05f)
    }
}
