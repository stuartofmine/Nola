package com.nola.ui.generative

import androidx.compose.ui.res.stringResource
import com.nola.R

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import com.nola.data.local.*
import kotlinx.serialization.json.*

/** All props/slots are resolved by Core. This component has no protocol runtime. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun GenerativeUiNode(
    id: String,
    snapshot: UiSnapshot,
    enabled: Boolean,
    value: (String, String, JsonElement?) -> JsonElement?,
    edit: (String, String, JsonElement) -> Unit,
    dispatch: (UiActionRef) -> Unit,
    present: (String) -> Unit,
    depth: Int = 0,
    presentationOnly: Boolean = false,
    composition: (String, Boolean) -> Unit = { _, _ -> },
    actionsEnabled: Boolean = enabled,
) {
    if (depth > 32) return
    val node = snapshot.nodes[id] ?: return
    val interactive = !presentationOnly && enabled && snapshot.phase == UiSurfacePhase.Ready && !node.flag("disabled", false)
    val editable = interactive && node.bindings.containsKey("value")
    val current = value(id, "value", node.props["value"])
    val label = node.text("label").ifEmpty { node.text("ariaLabel") }
    val child: @Composable (String) -> Unit = { childId ->
        key(childId) { GenerativeUiNode(childId, snapshot, enabled, value, edit, dispatch, present, depth + 1, presentationOnly, composition, actionsEnabled) }
    }
    val slot: @Composable (String) -> Unit = { name -> node.slots[name].orEmpty().forEach { child(it) } }
    val labelSlot: @Composable (String) -> Unit = { name ->
        node.slots[name].orEmpty().forEach { childId -> key(childId) {
            GenerativeUiNode(childId, snapshot, false, value, edit, {}, {}, depth + 1, true)
        } }
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        when (node.component) {
            "Text" -> Text(node.text("text"), style = when (node.text("variant")) {
                "h1" -> MaterialTheme.typography.headlineLarge
                "h2" -> MaterialTheme.typography.headlineMedium
                "h3" -> MaterialTheme.typography.headlineSmall
                "h4", "h5", "h6" -> MaterialTheme.typography.titleMedium
                "caption" -> MaterialTheme.typography.bodySmall
                else -> MaterialTheme.typography.bodyLarge
            })
            "Icon" -> GenerativeUiIcon(node.props["name"], node.text("ariaLabel"))
            "Row", "Column", "List" -> GenerativeUiLayout(node, snapshot, child)
            "Card" -> Surface(shape = MaterialTheme.shapes.medium,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)) {
                Column(Modifier.fillMaxWidth().padding(16.dp)) { slot("child") }
            }
            "Divider" -> HorizontalDivider()
            "Button" -> if (presentationOnly) labelSlot("child") else {
                val actionable = interactive && actionsEnabled && node.actions["action"] != null && node.validation_errors.isEmpty()
                val clicked = { node.actions["action"]?.let(dispatch); Unit }
                val modifier = Modifier.heightIn(min = 48.dp)
                when (node.text("variant")) {
                    "primary" -> Button(enabled = actionable, onClick = clicked, modifier = modifier) { labelSlot("child") }
                    "borderless" -> TextButton(enabled = actionable, onClick = clicked, modifier = modifier) { labelSlot("child") }
                    else -> OutlinedButton(enabled = actionable, onClick = clicked, modifier = modifier) { labelSlot("child") }
                }
            }
            "TextField" -> {
                val variant = node.text("variant")
                val committed = (current as? JsonPrimitive)?.contentOrNull.orEmpty()
                // Preserve the editing text; Core owns value validation.
                var text by remember(id) { mutableStateOf(TextFieldValue(committed)) }
                LaunchedEffect(committed) {
                    if (text.composition == null && committed != text.text) {
                        text = TextFieldValue(committed, TextRange(text.selection.start.coerceAtMost(committed.length),
                            text.selection.end.coerceAtMost(committed.length)))
                    }
                }
                DisposableEffect(id) { onDispose { composition(id, false) } }
                OutlinedTextField(value = text, enabled = editable, label = { Text(label) },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    singleLine = variant != "longText",
                    visualTransformation = if (variant == "obscured") PasswordVisualTransformation() else VisualTransformation.None,
                    keyboardOptions = KeyboardOptions(keyboardType = if (variant == "number") KeyboardType.Decimal else KeyboardType.Text),
                    onValueChange = { next ->
                        text = next
                        // v0.9.1 keeps numeric text fields as strings too.
                        composition(id, next.composition != null)
                        if (next.composition == null) edit(id, "value", JsonPrimitive(next.text))
                    })
            }
            "CheckBox" -> {
                val checked = (current as? JsonPrimitive)?.booleanOrNull ?: false
                Row(Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .clickable(enabled = editable) { edit(id, "value", JsonPrimitive(!checked)) }) {
                    Checkbox(checked = checked, enabled = editable, onCheckedChange = null)
                    Text(label, modifier = Modifier.padding(12.dp))
                }
            }
            "Slider" -> {
                val minimum = node.number("min", 0.0).toFloat()
                val maximum = node.number("max", 100.0).toFloat()
                if (minimum.isFinite() && maximum.isFinite() && minimum < maximum) {
                    Text(label)
                    Slider(enabled = editable,
                        value = ((current as? JsonPrimitive)?.floatOrNull ?: minimum).coerceIn(minimum, maximum),
                        onValueChange = { edit(id, "value", JsonPrimitive(it)) },
                        valueRange = minimum..maximum,
                        steps = (node.number("steps", 0.0).toInt() - 1).coerceIn(0, 10000),
                        modifier = Modifier.semantics { contentDescription = label })
                }
            }
            "ChoicePicker" -> {
                Text(label)
                val selected = (current as? JsonArray)?.toList().orEmpty()
                (node.props["options"] as? JsonArray).orEmpty().forEachIndexed { index, option ->
                    val fields = option as? JsonObject
                    val optionValue = fields?.get("value")
                    if (optionValue != null) key(index) {
                        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(enabled = editable) {
                            val next = if (node.text("variant") == "multipleSelection") selected.toMutableList() else mutableListOf()
                            if (optionValue in next) next.remove(optionValue) else next.add(optionValue)
                            edit(id, "value", JsonArray(next))
                        }) {
                            Checkbox(checked = optionValue in selected, enabled = editable, onCheckedChange = null)
                            Text((fields["label"] as? JsonPrimitive)?.contentOrNull.orEmpty(), Modifier.padding(12.dp))
                        }
                    }
                }
            }
            "DateTimeInput" -> GenerativeUiDateInput(label,
                (current as? JsonPrimitive)?.contentOrNull.orEmpty(),
                node.flag("enableDate", false), node.flag("enableTime", false), editable,
                changed = { edit(id, "value", JsonPrimitive(it)) })
            "Tabs" -> {
                val titles = (node.props["tab_titles"] as? JsonArray).orEmpty()
                var selected by remember(id) { mutableIntStateOf(0) }
                if (titles.isNotEmpty()) {
                    val selectedIndex = selected.coerceIn(titles.indices)
                    ScrollableTabRow(selectedTabIndex = selectedIndex) {
                        titles.forEachIndexed { index, title ->
                            Tab(selected = selectedIndex == index, onClick = { selected = index },
                                text = { Text((title as? JsonPrimitive)?.contentOrNull.orEmpty()) })
                        }
                    }
                    slot("tab:$selectedIndex")
                }
            }
            "Modal" -> if (presentationOnly) labelSlot("trigger") else TextButton(
                enabled = interactive, onClick = { present(id) }, modifier = Modifier.heightIn(min = 48.dp)
            ) { labelSlot("trigger") }
            else -> Text(stringResource(R.string.ui_component_unavailable, node.component), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        node.validation_errors.forEach { message -> Text(message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
    }
}

private fun UiNodeSnapshot.text(name: String): String = (props[name] as? JsonPrimitive)?.contentOrNull.orEmpty()
private fun UiNodeSnapshot.flag(name: String, default: Boolean): Boolean = (props[name] as? JsonPrimitive)?.booleanOrNull ?: default
private fun UiNodeSnapshot.number(name: String, default: Double): Double = (props[name] as? JsonPrimitive)?.doubleOrNull ?: default
