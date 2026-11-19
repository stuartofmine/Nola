package com.nola.ui.theme

import androidx.compose.ui.graphics.Color
import com.nola.ui.components.NolaColors
import java.io.File
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LightThemeContrastTest {
    private val json = Json { ignoreUnknownKeys = true }

    private fun loadPaletteTokens(): Map<String, Map<String, Color>> {
        val tokenFile = generateSequence(File(".").absoluteFile) { it.parentFile }
            .map { File(it, "shared/design-tokens/tokens.json") }
            .firstOrNull(File::isFile)
            ?: error("Could not locate shared/design-tokens/tokens.json from ${File(".").absolutePath}")
        val root = json.parseToJsonElement(tokenFile.readText()).jsonObject
        val colorRoot = root.getValue("color").jsonObject
        return listOf("light", "dark").associateWith { appearance ->
            colorRoot.getValue(appearance).jsonObject.mapValues { (_, element) ->
                parseHexColor(element.jsonPrimitive.content)
            }
        }
    }

    @Test
    fun themeSchemesAndNolaColorsMatchSharedWarmPaperAndWarmInkTokens() {
        val tokens = loadPaletteTokens()
        val light = tokens.getValue("light")
        val dark = tokens.getValue("dark")

        assertEquals(light.getValue("canvas"), NolaColors.bgLight)
        assertEquals(Color(0xFFF7F6F3), NolaColors.bgLight)
        assertEquals(light.getValue("canvas"), LightColors.background)
        assertEquals(light.getValue("surface"), LightColors.surface)
        assertEquals(light.getValue("surfaceSubtle"), LightColors.surfaceContainer)
        assertEquals(light.getValue("surfaceSubtle"), LightColors.surfaceVariant)
        assertEquals(light.getValue("ink"), LightColors.onBackground)
        assertEquals(light.getValue("ink"), LightColors.onSurface)
        assertEquals(light.getValue("inkSecondary"), LightColors.onSurfaceVariant)
        assertEquals(light.getValue("accent"), LightColors.primary)
        assertEquals(light.getValue("onAccent"), LightColors.onPrimary)
        assertEquals(light.getValue("accentSubtle"), LightColors.primaryContainer)
        assertEquals(light.getValue("warning"), LightColors.tertiary)
        assertEquals(light.getValue("danger"), LightColors.error)
        assertEquals(light.getValue("outline"), LightColors.outline)
        assertEquals(light.getValue("border"), LightColors.outlineVariant)

        assertEquals(dark.getValue("canvas"), NolaColors.bgDark)
        assertEquals(Color(0xFF131312), NolaColors.bgDark)
        assertEquals(dark.getValue("canvas"), DarkColors.background)
        assertEquals(dark.getValue("surface"), DarkColors.surface)
        assertEquals(dark.getValue("surfaceSubtle"), DarkColors.surfaceContainer)
        assertEquals(dark.getValue("surfaceSubtle"), DarkColors.surfaceVariant)
        assertEquals(dark.getValue("ink"), DarkColors.onBackground)
        assertEquals(dark.getValue("ink"), DarkColors.onSurface)
        assertEquals(dark.getValue("inkSecondary"), DarkColors.onSurfaceVariant)
        assertEquals(dark.getValue("accent"), DarkColors.primary)
        assertEquals(dark.getValue("onAccent"), DarkColors.onPrimary)
        assertEquals(dark.getValue("accentSubtle"), DarkColors.primaryContainer)
        assertEquals(dark.getValue("warning"), DarkColors.tertiary)
        assertEquals(dark.getValue("danger"), DarkColors.error)
        assertEquals(dark.getValue("outline"), DarkColors.outline)
        assertEquals(dark.getValue("border"), DarkColors.outlineVariant)
    }

    @Test
    fun warmPaperAndWarmInkForbidPureOledBlackAndNeutralColdGrays() {
        val neutralSurfaces = listOf(
            "LightColors.background" to LightColors.background,
            "LightColors.surfaceContainer" to LightColors.surfaceContainer,
            "LightColors.primaryContainer" to LightColors.primaryContainer,
            "LightColors.outlineVariant" to LightColors.outlineVariant,
            "DarkColors.background" to DarkColors.background,
            "DarkColors.surface" to DarkColors.surface,
            "DarkColors.surfaceContainer" to DarkColors.surfaceContainer,
            "DarkColors.primaryContainer" to DarkColors.primaryContainer,
            "DarkColors.outlineVariant" to DarkColors.outlineVariant,
            "DarkColors.onBackground" to DarkColors.onBackground,
            "DarkColors.onSurface" to DarkColors.onSurface,
        )
        neutralSurfaces.forEach { (label, color) ->
            assertNotEquals("$label must never be pure OLED black", Color(0xFF000000), color)
            val isNeutralColdGray = color.red == color.green && color.green == color.blue
            assertTrue("$label ($color) must carry warm undertone rather than pure cold gray", !isNeutralColdGray)
        }
        assertNotEquals(Color(0xFF000000), NolaColors.bgDark)
        assertNotEquals(Color(0xFF121212), NolaColors.bgDark)
        assertNotEquals(Color(0xFFFFFFFF), NolaColors.bgLight)
    }

    @Test
    fun semanticAndAlphaCompositedPairsMeetWcagAaContrast() {
        val lightSuccess = Color(0xFF237047)
        val darkSuccess = Color(0xFF82C99B)

        val pairs = listOf(
            // Light mode semantic pairs
            "light onBackground on background" to (LightColors.onBackground to LightColors.background),
            "light onSurface on surface" to (LightColors.onSurface to LightColors.surface),
            "light onSurfaceVariant on background" to (LightColors.onSurfaceVariant to LightColors.background),
            "light onSurfaceVariant on surface" to (LightColors.onSurfaceVariant to LightColors.surface),
            "light onSurfaceVariant on surfaceContainer" to (LightColors.onSurfaceVariant to LightColors.surfaceContainer),
            "light onPrimary on primary" to (LightColors.onPrimary to LightColors.primary),
            "light onPrimaryContainer on primaryContainer" to (LightColors.onPrimaryContainer to LightColors.primaryContainer),
            "light onSecondaryContainer on secondaryContainer" to (LightColors.onSecondaryContainer to LightColors.secondaryContainer),
            "light onTertiary on tertiary" to (LightColors.onTertiary to LightColors.tertiary),
            "light primary on surface" to (LightColors.primary to LightColors.surface),
            "light primary on surfaceContainer" to (LightColors.primary to LightColors.surfaceContainer),
            "light tertiary on surface" to (LightColors.tertiary to LightColors.surface),
            "light error on background" to (LightColors.error to LightColors.background),
            "light error on surface" to (LightColors.error to LightColors.surface),
            "light error on surfaceContainer" to (LightColors.error to LightColors.surfaceContainer),
            "light success on background" to (lightSuccess to LightColors.background),
            "light success on surface" to (lightSuccess to LightColors.surface),

            // Dark mode semantic pairs
            "dark onBackground on background" to (DarkColors.onBackground to DarkColors.background),
            "dark onSurface on surface" to (DarkColors.onSurface to DarkColors.surface),
            "dark onSurfaceVariant on background" to (DarkColors.onSurfaceVariant to DarkColors.background),
            "dark onSurfaceVariant on surface" to (DarkColors.onSurfaceVariant to DarkColors.surface),
            "dark onSurfaceVariant on surfaceContainer" to (DarkColors.onSurfaceVariant to DarkColors.surfaceContainer),
            "dark onPrimary on primary" to (DarkColors.onPrimary to DarkColors.primary),
            "dark onPrimaryContainer on primaryContainer" to (DarkColors.onPrimaryContainer to DarkColors.primaryContainer),
            "dark onSecondaryContainer on secondaryContainer" to (DarkColors.onSecondaryContainer to DarkColors.secondaryContainer),
            "dark onTertiary on tertiary" to (DarkColors.onTertiary to DarkColors.tertiary),
            "dark primary on surface" to (DarkColors.primary to DarkColors.surface),
            "dark primary on surfaceContainer" to (DarkColors.primary to DarkColors.surfaceContainer),
            "dark tertiary on surface" to (DarkColors.tertiary to DarkColors.surface),
            "dark error on background" to (DarkColors.error to DarkColors.background),
            "dark error on surface" to (DarkColors.error to DarkColors.surface),
            "dark error on surfaceContainer" to (DarkColors.error to DarkColors.surfaceContainer),
            "dark success on background" to (darkSuccess to DarkColors.background),
            "dark success on surface" to (darkSuccess to DarkColors.surface),

            // Alpha-composited fill pairs used in NolaChip, NolaBadge, GuideNote, TaskRow
            "light onSurfaceVariant on surfaceVariant@50% over surface" to (
                LightColors.onSurfaceVariant to compositeOver(LightColors.surfaceVariant.copy(alpha = 0.50f), LightColors.surface)
                ),
            "light onSurfaceVariant on surfaceVariant@34% over background" to (
                LightColors.onSurfaceVariant to compositeOver(LightColors.surfaceVariant.copy(alpha = 0.34f), LightColors.background)
                ),
            "light primary on primaryContainer@60% over background" to (
                LightColors.primary to compositeOver(LightColors.primaryContainer.copy(alpha = 0.60f), LightColors.background)
                ),
            "light primary on primary@12% over surface" to (
                LightColors.primary to compositeOver(LightColors.primary.copy(alpha = 0.12f), LightColors.surface)
                ),
            "light error on error@12% over surface" to (
                LightColors.error to compositeOver(LightColors.error.copy(alpha = 0.12f), LightColors.surface)
                ),
            "light success on success@12% over surface" to (
                lightSuccess to compositeOver(lightSuccess.copy(alpha = 0.12f), LightColors.surface)
                ),
            "light warning on warning@12% over surface" to (
                LightColors.tertiary to compositeOver(LightColors.tertiary.copy(alpha = 0.12f), LightColors.surface)
                ),
            "dark onSurfaceVariant on surfaceVariant@50% over surface" to (
                DarkColors.onSurfaceVariant to compositeOver(DarkColors.surfaceVariant.copy(alpha = 0.50f), DarkColors.surface)
                ),
            "dark onSurfaceVariant on surfaceVariant@34% over background" to (
                DarkColors.onSurfaceVariant to compositeOver(DarkColors.surfaceVariant.copy(alpha = 0.34f), DarkColors.background)
                ),
            "dark primary on primaryContainer@60% over background" to (
                DarkColors.primary to compositeOver(DarkColors.primaryContainer.copy(alpha = 0.60f), DarkColors.background)
                ),
            "dark primary on primary@12% over surface" to (
                DarkColors.primary to compositeOver(DarkColors.primary.copy(alpha = 0.12f), DarkColors.surface)
                ),
            "dark error on error@12% over surface" to (
                DarkColors.error to compositeOver(DarkColors.error.copy(alpha = 0.12f), DarkColors.surface)
                ),
            "dark success on success@12% over surface" to (
                darkSuccess to compositeOver(darkSuccess.copy(alpha = 0.12f), DarkColors.surface)
                ),
            "dark warning on warning@12% over surface" to (
                DarkColors.tertiary to compositeOver(DarkColors.tertiary.copy(alpha = 0.12f), DarkColors.surface)
                ),
        )

        pairs.forEach { (label, pair) ->
            val ratio = contrastRatio(pair.first, pair.second)
            val ratioText = "%.2f".format(ratio)
            assertTrue("$label contrast $ratioText is below 4.5:1", ratio >= 4.5)
        }
    }

    private fun parseHexColor(hex: String): Color {
        val cleaned = hex.trim().removePrefix("#")
        require(cleaned.length == 6) { "Expected 6-digit hex color, got: $hex" }
        val rgb = cleaned.toLong(16)
        return Color(0xFF000000L or rgb)
    }

    private fun compositeOver(foreground: Color, background: Color): Color {
        val a = foreground.alpha
        val invA = 1f - a
        return Color(
            red = foreground.red * a + background.red * invA,
            green = foreground.green * a + background.green * invA,
            blue = foreground.blue * a + background.blue * invA,
            alpha = 1f,
        )
    }

    private fun contrastRatio(first: Color, second: Color): Double {
        val a = luminance(first)
        val b = luminance(second)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    private fun luminance(color: Color): Double {
        fun linear(component: Float): Double {
            val value = component.toDouble()
            return if (value <= 0.04045) value / 12.92 else ((value + 0.055) / 1.055).pow(2.4)
        }
        return linear(color.red) * 0.2126 + linear(color.green) * 0.7152 + linear(color.blue) * 0.0722
    }
}
