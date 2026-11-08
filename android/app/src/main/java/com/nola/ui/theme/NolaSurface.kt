package com.nola.ui.theme

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.CornerSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp

/** Two tangent cubic segments per corner, with zero curvature at each straight
 * edge and matching curvature at their midpoint. No circular-arc/line kink. */
class NolaContinuousShape(
    topStart: CornerSize, topEnd: CornerSize, bottomEnd: CornerSize, bottomStart: CornerSize
) : CornerBasedShape(topStart, topEnd, bottomEnd, bottomStart) {
    constructor(radius: Dp) : this(CornerSize(radius), CornerSize(radius), CornerSize(radius), CornerSize(radius))

    override fun copy(topStart: CornerSize, topEnd: CornerSize, bottomEnd: CornerSize, bottomStart: CornerSize) =
        NolaContinuousShape(topStart, topEnd, bottomEnd, bottomStart)

    override fun createOutline(size: Size, topStart: Float, topEnd: Float, bottomEnd: Float,
                               bottomStart: Float, layoutDirection: LayoutDirection): Outline {
        val rtl = layoutDirection == LayoutDirection.Rtl
        val tl = if (rtl) topEnd else topStart
        val tr = if (rtl) topStart else topEnd
        val br = if (rtl) bottomStart else bottomEnd
        val bl = if (rtl) bottomEnd else bottomStart
        val w = size.width; val h = size.height
        return Outline.Generic(Path().apply {
            moveTo(tl, 0f); lineTo(w - tr, 0f)
            cubicTo(w - tr * .55f, 0f, w - tr * .2f, 0f, w - tr * .1f, tr * .1f)
            cubicTo(w, tr * .2f, w, tr * .55f, w, tr)
            lineTo(w, h - br)
            cubicTo(w, h - br * .55f, w, h - br * .2f, w - br * .1f, h - br * .1f)
            cubicTo(w - br * .2f, h, w - br * .55f, h, w - br, h)
            lineTo(bl, h)
            cubicTo(bl * .55f, h, bl * .2f, h, bl * .1f, h - bl * .1f)
            cubicTo(0f, h - bl * .2f, 0f, h - bl * .55f, 0f, h - bl)
            lineTo(0f, tl)
            cubicTo(0f, tl * .55f, 0f, tl * .2f, tl * .1f, tl * .1f)
            cubicTo(tl * .2f, 0f, tl * .55f, 0f, tl, 0f)
            close()
        })
    }
}

/** Shared opaque card/control material; only its background casts a shadow. */
@Composable
fun Modifier.nolaSurface(radius: Dp = 16.dp, raised: Boolean = true,
                         fill: Color = MaterialTheme.colorScheme.surface,
                         border: Color = MaterialTheme.colorScheme.outlineVariant): Modifier {
    val shape = remember(radius) { NolaContinuousShape(radius) }
    val dark = MaterialTheme.colorScheme.background.luminance() < .5f
    val top = androidx.compose.ui.graphics.lerp(fill, Color.White, if (dark) .045f else .34f)
    val bottom = androidx.compose.ui.graphics.lerp(fill, Color.Black, if (dark) .025f else .018f)
    return shadow(if (raised) 3.dp else 1.dp, shape, clip = false,
        ambientColor = Color.Black.copy(alpha = .10f), spotColor = Color.Black.copy(alpha = .10f))
        .background(Brush.verticalGradient(listOf(top, bottom)), shape)
        .border(.75.dp, border, shape)
        .clip(shape)
        .drawWithCache {
            val inset = 1.25.dp.toPx()
            val outline = shape.createOutline(Size((size.width - 2 * inset).coerceAtLeast(0f),
                (size.height - 2 * inset).coerceAtLeast(0f)), layoutDirection, this)
            val highlight = Brush.verticalGradient(listOf(Color.White.copy(alpha = if (dark) .09f else .65f), Color.Transparent))
            onDrawWithContent {
                drawContent()
                translate(inset, inset) { drawOutline(outline, highlight, style = Stroke(.75.dp.toPx())) }
            }
        }
}
