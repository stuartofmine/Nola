package com.nola.ui.generative

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import com.nola.data.local.UiNodeSnapshot
import com.nola.data.local.UiSnapshot
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull

internal fun horizontalArrangement(justify: String): Arrangement.Horizontal = when (justify) {
    "center" -> Arrangement.Center
    "end" -> Arrangement.End
    "spaceBetween" -> Arrangement.SpaceBetween
    "spaceAround" -> Arrangement.SpaceAround
    "spaceEvenly" -> Arrangement.SpaceEvenly
    else -> Arrangement.Start
}
internal fun verticalArrangement(justify: String): Arrangement.Vertical = when (justify) {
    "center" -> Arrangement.Center
    "end" -> Arrangement.Bottom
    "spaceBetween" -> Arrangement.SpaceBetween
    "spaceAround" -> Arrangement.SpaceAround
    "spaceEvenly" -> Arrangement.SpaceEvenly
    else -> Arrangement.Top
}
internal fun UiNodeSnapshot.layoutText(property: String, default: String): String =
    (props[property] as? JsonPrimitive)?.contentOrNull ?: default
internal fun UiNodeSnapshot.layoutWeight(): Double? =
    (props["weight"] as? JsonPrimitive)?.doubleOrNull?.takeIf { it.isFinite() && it > 0.0 }

internal fun normalizedLayoutWeights(nodes: List<UiNodeSnapshot?>, stretch: Boolean): List<Float?> {
    val values = nodes.map { it?.layoutWeight() ?: if (stretch) 1.0 else null }
    val maximum = values.filterNotNull().maxOrNull() ?: return values.map { null }
    // Ratios preserve proportions without overflowing Compose's Float accumulator.
    return values.map { value -> value?.let { (it / maximum).toFloat().coerceAtLeast(Float.MIN_VALUE) } }
}

/** Native layout of an already resolved Core child slot. No binding or protocol evaluation. */
@Composable
internal fun GenerativeUiLayout(node: UiNodeSnapshot, snapshot: UiSnapshot, child: @Composable (String) -> Unit) {
    val horizontal = node.component == "Row" || node.component == "List" && node.layoutText("direction", "vertical") == "horizontal"
    val justify = node.layoutText("justify", "start")
    val align = node.layoutText("align", "stretch")
    val ids = node.slots["children"].orEmpty()
    val weights = normalizedLayoutWeights(ids.map(snapshot.nodes::get), justify == "stretch")
    if (horizontal) {
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min),
            horizontalArrangement = horizontalArrangement(justify),
            verticalAlignment = when (align) { "center" -> Alignment.CenterVertically; "end" -> Alignment.Bottom; else -> Alignment.Top }) {
            ids.forEachIndexed { index, id -> key(id) {
                val weight = weights[index]
                var modifier: Modifier = Modifier
                if (weight != null) modifier = modifier.weight(weight)
                if (align == "stretch") modifier = modifier.fillMaxHeight()
                Box(modifier, propagateMinConstraints = true) { child(id) }
            } }
        }
    } else {
        Column(Modifier.fillMaxWidth(), verticalArrangement = verticalArrangement(justify),
            horizontalAlignment = when (align) { "center" -> Alignment.CenterHorizontally; "end" -> Alignment.End; else -> Alignment.Start }) {
            ids.forEachIndexed { index, id -> key(id) {
                val weight = weights[index]
                var modifier: Modifier = Modifier
                if (weight != null) modifier = modifier.weight(weight)
                if (align == "stretch") modifier = modifier.fillMaxWidth()
                Box(modifier, propagateMinConstraints = true) { child(id) }
            } }
        }
    }
}
