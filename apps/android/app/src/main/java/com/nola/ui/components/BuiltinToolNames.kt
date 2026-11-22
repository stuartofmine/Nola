package com.nola.ui.components

import java.util.Locale
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

internal object BuiltinToolNames {
    private val catalog by lazy {
        val resource = requireNotNull(javaClass.getResourceAsStream("/tool-names.json"))
        resource.bufferedReader().use { Json.parseToJsonElement(it.readText()).jsonObject }
    }

    fun label(id: String, title: String?, locale: Locale): String? {
        val key = id.trim().lowercase(Locale.ROOT).replace('-', '_')
        val aliases = catalog.getValue("aliases").jsonObject
        val canonical = aliases[key]?.jsonPrimitive?.content ?: key
        val labels = catalog.getValue("labels").jsonObject[canonical]?.jsonObject ?: return null
        val declared = title.orEmpty().trim()
        val titleKey = declared.lowercase(Locale.ROOT).replace(' ', '_').replace('-', '_')
        if (declared.isNotEmpty() && declared != "Tool" &&
            (aliases[titleKey]?.jsonPrimitive?.content ?: titleKey) != canonical &&
            labels.values.none { it.jsonPrimitive.content == declared }) return null
        val language = when {
            locale.language != "zh" -> locale.language
            locale.script == "Hant" || locale.country in setOf("TW", "HK", "MO") -> "zh-Hant"
            else -> "zh-Hans"
        }
        return (labels[language] ?: labels.getValue("en")).jsonPrimitive.content
    }
}
