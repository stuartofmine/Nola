package com.nola.data.local

import com.nola.domain.model.UploadedContent
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** A malformed projection is an error, never a partially successful catalog. */
internal fun decodeAndroidUserSkills(value: JsonElement): List<UploadedContent> =
    value.extractJsonArray("skills", "items", "records").mapNotNull { entry ->
        val envelope = entry.jsonObject
        if (envelope.containsKey("record")) {
            val record = envelope.getValue("record").jsonObject
            val revision = envelope.getValue("revision").jsonPrimitive.content.toLong()
            require(revision > 0) { "Skill editor revision must be positive" }
            val source = record.getValue("source").jsonPrimitive.content
            when (if (source == "builtin") "built_in" else source) {
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
        } else if (envelope.containsKey("name")) {
            val rawName = envelope.getValue("name").jsonPrimitive.content
            val rawSource = envelope["source"]?.jsonPrimitive?.contentOrNull ?: "user"
            when (if (rawSource == "builtin") "built_in" else rawSource) {
                "built_in" -> null
                "user" -> {
                    val revision = envelope["revision"]?.jsonPrimitive?.contentOrNull?.toLong() ?: 1L
                    require(revision > 0) { "Skill editor revision must be positive" }
                    val id = (envelope["id"]?.jsonPrimitive?.contentOrNull ?: rawName)
                        .also { require(it.isNotBlank()) }
                    val displayName = envelope["display_name"]?.jsonPrimitive?.contentOrNull ?: rawName
                    val instructions = envelope["instructions_text"]?.jsonPrimitive?.contentOrNull
                        ?: envelope["body"]?.jsonPrimitive?.contentOrNull
                        ?: ""
                    val enabled = envelope["enabled"]?.jsonPrimitive?.contentOrNull?.toBooleanStrict() ?: true
                    val summary = envelope["summary"]?.jsonPrimitive?.contentOrNull
                        ?: envelope["description"]?.jsonPrimitive?.contentOrNull
                        ?: ""
                    val version = envelope["version"]?.jsonPrimitive?.contentOrNull ?: "1.0"
                    val requiredTools = (envelope["required_tools"] as? JsonArray).orEmpty().mapNotNull { reference ->
                        when (reference) {
                            is JsonPrimitive -> reference.contentOrNull
                            is JsonObject -> if (reference["provider"]?.jsonPrimitive?.contentOrNull == "builtin") {
                                reference["tool_name"]?.jsonPrimitive?.contentOrNull
                            } else null
                            else -> null
                        }
                    }
                    UploadedContent(
                        canonicalRevision = revision,
                        canonicalSkillRecord = envelope,
                        name = displayName,
                        content = instructions,
                        enabled = enabled,
                        description = summary,
                        version = version,
                        requiredTools = requiredTools,
                        resourceId = id,
                    )
                }
                else -> error("Unsupported canonical Skill source: $rawSource")
            }
        } else {
            error("Canonical Skill editor snapshot requires record and revision")
        }
    }
