package com.nola.data.local

import com.nola.domain.model.UploadedContent
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** A malformed projection is an error, never a partially successful catalog. */
internal fun decodeAndroidUserSkills(value: JsonElement): List<UploadedContent> =
    value.jsonArray.mapNotNull { entry ->
        val envelope = entry.jsonObject
        val record = envelope.getValue("record").jsonObject
        val revision = envelope.getValue("revision").jsonPrimitive.content.toLong()
        require(revision > 0) { "Skill editor revision must be positive" }
        when (val source = record.getValue("source").jsonPrimitive.content) {
            "built_in" -> null
            "user" -> UploadedContent(
                canonicalRevision = revision,
                canonicalSkillRecord = record,
                name = record.getValue("display_name").jsonPrimitive.content,
                content = record.getValue("instructions_text").jsonPrimitive.content,
                enabled = record.getValue("enabled").jsonPrimitive.content.toBooleanStrict(),
                description = record.getValue("summary").jsonPrimitive.content,
                version = record.getValue("version").jsonPrimitive.content,
                // This display model lists builtin names only. Editors use the full
                // canonical snapshot and must not serialize this list back to Core.
                requiredTools = record.getValue("required_tools").jsonArray.mapNotNull { reference ->
                    val tool = reference.jsonObject
                    if (tool.getValue("provider").jsonPrimitive.content == "builtin") {
                        tool.getValue("tool_name").jsonPrimitive.content
                    } else null
                },
                resourceId = record.getValue("id").jsonPrimitive.content.also { require(it.isNotBlank()) },
            )
            else -> error("Unsupported canonical Skill source: $source")
        }
    }
