// SPDX-License-Identifier: Apache-2.0
package com.nola.ui

import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.PixelMap
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.text.TextLayoutResult
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue

internal object WarmPaletteExpectations {
    val LightBackground = Color(0xFFF7F6F3)
    val LightSurface = Color(0xFFFFFFFF)
    val LightSurfaceContainer = Color(0xFFF1F0EC)
    val LightText = Color(0xFF181817)
    val LightSecondaryText = Color(0xFF5C5B57)

    val DarkBackground = Color(0xFF131312)
    val DarkSurface = Color(0xFF1B1B19)
    val DarkSurfaceContainer = Color(0xFF232320)
    val DarkText = Color(0xFFF2F0EB)
    val DarkSecondaryText = Color(0xFFA3A09A)

    val LightSurfaces = listOf(LightBackground, LightSurface, LightSurfaceContainer)
    val DarkSurfaces = listOf(DarkBackground, DarkSurface, DarkSurfaceContainer)
}

internal fun colorsMatchWithinTolerance(
    actual: Color,
    expected: Color,
    channelTolerance: Int = 6,
): Boolean {
    val r1 = (actual.red * 255f).roundToInt()
    val g1 = (actual.green * 255f).roundToInt()
    val b1 = (actual.blue * 255f).roundToInt()
    val r2 = (expected.red * 255f).roundToInt()
    val g2 = (expected.green * 255f).roundToInt()
    val b2 = (expected.blue * 255f).roundToInt()
    return abs(r1 - r2) <= channelTolerance &&
        abs(g1 - g2) <= channelTolerance &&
        abs(b1 - b2) <= channelTolerance
}

internal fun Color.toHexRgb(): String {
    val r = (red * 255f).roundToInt().coerceIn(0, 255)
    val g = (green * 255f).roundToInt().coerceIn(0, 255)
    val b = (blue * 255f).roundToInt().coerceIn(0, 255)
    return "#%02X%02X%02X".format(r, g, b)
}

internal fun PixelMap.sampledColorsGrid(
    columns: Int = 12,
    rows: Int = 12,
    insetFraction: Float = 0.08f,
): List<Color> {
    if (width <= 0 || height <= 0) return emptyList()
    val minX = (width * insetFraction).toInt().coerceIn(0, width - 1)
    val maxX = (width * (1f - insetFraction)).toInt().coerceIn(minX, width - 1)
    val minY = (height * insetFraction).toInt().coerceIn(0, height - 1)
    val maxY = (height * (1f - insetFraction)).toInt().coerceIn(minY, height - 1)
    val result = ArrayList<Color>(columns * rows)
    for (row in 0 until rows) {
        val y = if (rows == 1) (minY + maxY) / 2 else minY + (maxY - minY) * row / (rows - 1)
        for (col in 0 until columns) {
            val x = if (columns == 1) (minX + maxX) / 2 else minX + (maxX - minX) * col / (columns - 1)
            result += this[x.coerceIn(0, width - 1), y.coerceIn(0, height - 1)]
        }
    }
    return result
}

internal fun PixelMap.dominantInteriorColor(
    columns: Int = 16,
    rows: Int = 16,
    insetFraction: Float = 0.06f,
): Color {
    val samples = sampledColorsGrid(columns = columns, rows = rows, insetFraction = insetFraction)
    check(samples.isNotEmpty()) { "Captured PixelMap is empty (${width}x$height)" }
    val buckets = linkedMapOf<Int, MutableList<Color>>()
    for (sample in samples) {
        val rq = ((sample.red * 255f).roundToInt() / 4) * 4
        val gq = ((sample.green * 255f).roundToInt() / 4) * 4
        val bq = ((sample.blue * 255f).roundToInt() / 4) * 4
        val key = (rq shl 16) or (gq shl 8) or bq
        buckets.getOrPut(key) { mutableListOf() }.add(sample)
    }
    val largest = buckets.values.maxByOrNull { it.size }!!
    return Color(
        red = largest.map { it.red }.average().toFloat(),
        green = largest.map { it.green }.average().toFloat(),
        blue = largest.map { it.blue }.average().toFloat(),
        alpha = 1f,
    )
}

internal fun PixelMap.assertDominantWarmSurface(
    darkTheme: Boolean,
    expectedCandidates: List<Color> = if (darkTheme) WarmPaletteExpectations.DarkSurfaces else WarmPaletteExpectations.LightSurfaces,
    label: String,
    channelTolerance: Int = 6,
) {
    val dominant = dominantInteriorColor()
    assertTrue(
        "$label dominant background ${dominant.toHexRgb()} did not match expected ${expectedCandidates.map { it.toHexRgb() }}",
        expectedCandidates.any { colorsMatchWithinTolerance(dominant, it, channelTolerance) },
    )
    assertZeroOledOrCoolGrayLeaks(darkTheme = darkTheme, label = label)
}

internal fun PixelMap.assertZeroOledOrCoolGrayLeaks(
    darkTheme: Boolean,
    label: String,
) {
    val samples = sampledColorsGrid(columns = 16, rows = 16, insetFraction = 0.06f)
    for (sample in samples) {
        val r = (sample.red * 255f).roundToInt()
        val g = (sample.green * 255f).roundToInt()
        val b = (sample.blue * 255f).roundToInt()
        assertFalse(
            "$label leaked pure OLED black pixel ${sample.toHexRgb()}",
            r <= 4 && g <= 4 && b <= 4,
        )
        if (darkTheme) {
            val hasCoolBlueBias = b > r && b > g
            val isM3DefaultCoolDark = hasCoolBlueBias && (
                colorsMatchWithinTolerance(sample, Color(0xFF1C1B1F), 3) ||
                    colorsMatchWithinTolerance(sample, Color(0xFF141218), 3) ||
                    colorsMatchWithinTolerance(sample, Color(0xFF211F26), 3)
                )
            assertFalse(
                "$label leaked Material 3 default cool/purple dark surface ${sample.toHexRgb()}",
                isM3DefaultCoolDark,
            )
        }
    }
}

internal fun SemanticsNodeInteraction.assertRenderedBackgroundMatches(
    expected: Color,
    darkTheme: Boolean,
    label: String,
    channelTolerance: Int = 6,
): PixelMap {
    val pixelMap = captureToImage().toPixelMap()
    val dominant = pixelMap.dominantInteriorColor()
    assertTrue(
        "$label expected background ${expected.toHexRgb()}, but rendered ${dominant.toHexRgb()}",
        colorsMatchWithinTolerance(dominant, expected, channelTolerance),
    )
    pixelMap.assertZeroOledOrCoolGrayLeaks(darkTheme = darkTheme, label = label)
    return pixelMap
}

internal fun ImageBitmap.assertWarmThemeSurface(
    darkTheme: Boolean,
    label: String,
    expectedCandidates: List<Color> = if (darkTheme) WarmPaletteExpectations.DarkSurfaces else WarmPaletteExpectations.LightSurfaces,
): PixelMap {
    val pixelMap = toPixelMap()
    pixelMap.assertDominantWarmSurface(
        darkTheme = darkTheme,
        expectedCandidates = expectedCandidates,
        label = label,
    )
    val dominant = pixelMap.dominantInteriorColor()
    val expectedPrimaryInk = if (darkTheme) WarmPaletteExpectations.DarkText else WarmPaletteExpectations.LightText
    val expectedSecondaryInk = if (darkTheme) WarmPaletteExpectations.DarkSecondaryText else WarmPaletteExpectations.LightSecondaryText
    val primaryRatio = wcagContrastRatio(expectedPrimaryInk, dominant)
    val secondaryRatio = wcagContrastRatio(expectedSecondaryInk, dominant)
    assertTrue(
        "$label Warm primary ink vs rendered surface contrast $primaryRatio is below 4.5:1",
        primaryRatio >= 4.5f,
    )
    assertTrue(
        "$label Warm secondary ink vs rendered surface contrast $secondaryRatio is below 4.5:1",
        secondaryRatio >= 4.5f,
    )
    return pixelMap
}

internal fun SemanticsNodeInteraction.fetchTextLayoutResult(): TextLayoutResult {
    val node = fetchSemanticsNode()
    val results = mutableListOf<TextLayoutResult>()
    val action = node.config.getOrNull(SemanticsActions.GetTextLayoutResult)
        ?: error("Semantics node does not expose GetTextLayoutResult")
    val ok = action.action?.invoke(results) == true
    check(ok && results.isNotEmpty()) { "GetTextLayoutResult returned no results" }
    return results.first()
}

internal fun SemanticsNodeInteraction.assertSingleLineNoOverflow(label: String): TextLayoutResult {
    val layout = fetchTextLayoutResult()
    assertEquals("$label must fit on a single line", 1, layout.lineCount)
    assertFalse("$label must not be ellipsized", layout.isLineEllipsized(0))
    val lineRight = layout.getLineRight(0)
    val lineBottom = layout.getLineBottom(0)
    assertTrue(
        "$label must not overflow horizontally (lineRight=$lineRight, width=${layout.size.width})",
        lineRight <= layout.size.width + 1f,
    )
    assertTrue(
        "$label must not overflow vertically (lineBottom=$lineBottom, height=${layout.size.height})",
        lineBottom <= layout.size.height + 1f,
    )
    return layout
}

internal fun SemanticsNodeInteraction.assertRenderedTextColor(
    expected: Color,
    label: String,
    channelTolerance: Int = 6,
): TextLayoutResult {
    val layout = fetchTextLayoutResult()
    val actual = layout.layoutInput.style.color
    assertTrue(
        "$label expected text color ${expected.toHexRgb()}, got ${actual.toHexRgb()} (alpha=${actual.alpha})",
        actual != Color.Unspecified && colorsMatchWithinTolerance(actual, expected, channelTolerance),
    )
    return layout
}

internal fun SemanticsNodeInteraction.assertNonBlackDarkText(label: String): TextLayoutResult {
    val layout = fetchTextLayoutResult()
    val actual = layout.layoutInput.style.color
    assertTrue("$label text color must be specified", actual != Color.Unspecified)
    assertFalse(
        "$label dark-mode text must never be near-black (${actual.toHexRgb()})",
        colorsMatchWithinTolerance(actual, Color.Black, 12),
    )
    assertTrue(
        "$label dark-mode text ${actual.toHexRgb()} has insufficient luminance (${actual.luminance()})",
        actual.luminance() > 0.15f,
    )
    return layout
}

internal fun wcagContrastRatio(first: Color, second: Color): Float {
    val a = first.luminance()
    val b = second.luminance()
    return (max(a, b) + 0.05f) / (min(a, b) + 0.05f)
}

internal fun SemanticsNodeInteraction.assertFullyInside(
    containerBounds: Rect,
    label: String,
    tolerancePx: Float = 1.5f,
) {
    val bounds = fetchSemanticsNode().boundsInRoot
    assertTrue(
        "$label left (${bounds.left}) clipped outside container (${containerBounds.left})",
        bounds.left >= containerBounds.left - tolerancePx,
    )
    assertTrue(
        "$label top (${bounds.top}) clipped outside container (${containerBounds.top})",
        bounds.top >= containerBounds.top - tolerancePx,
    )
    assertTrue(
        "$label right (${bounds.right}) overflows container (${containerBounds.right})",
        bounds.right <= containerBounds.right + tolerancePx,
    )
    assertTrue(
        "$label bottom (${bounds.bottom}) overflows container (${containerBounds.bottom})",
        bounds.bottom <= containerBounds.bottom + tolerancePx,
    )
}

internal fun SemanticsNodeInteraction.assertAboveBottomInset(
    containerBottomPx: Float,
    bottomInsetPx: Float,
    label: String,
    tolerancePx: Float = 1.5f,
) {
    val bounds = fetchSemanticsNode().boundsInRoot
    val maxAllowedBottom = containerBottomPx - bottomInsetPx + tolerancePx
    assertTrue(
        "$label top (${bounds.top}) must be >= 0",
        bounds.top >= -tolerancePx,
    )
    assertTrue(
        "$label bottom (${bounds.bottom}) overlapped bottom inset (maxAllowed=$maxAllowedBottom, containerBottom=$containerBottomPx, inset=$bottomInsetPx)",
        bounds.bottom <= maxAllowedBottom,
    )
}
