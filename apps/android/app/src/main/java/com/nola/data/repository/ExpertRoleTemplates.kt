// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable data class ExpertRoleSkill(val id: String, val name: Map<String, String>, val instructions: String)
@Serializable data class ExpertRoleToolPack(val id: String, val tools: List<String>)
@Serializable data class ExpertRoleTemplate(val id: String, @SerialName("role_id") val roleId: String,
    val name: Map<String, String>, val description: Map<String, String>, val prompt: String,
    @SerialName("tool_pack") val toolPack: ExpertRoleToolPack, val skills: List<ExpertRoleSkill>)
@Serializable data class ExpertRoleFeatures(@SerialName("reasoning_effort") val reasoningEffort: Boolean,
    @SerialName("explicit_generation_reasoning") val explicitGenerationReasoning: Boolean, val rejoin: Boolean,
    @SerialName("main_agent_delegation") val mainAgentDelegation: Boolean = false)
@Serializable data class ExpertRoleCatalog(val version: Int, val features: ExpertRoleFeatures, val templates: List<ExpertRoleTemplate>) {
    fun validated(): ExpertRoleCatalog {
        require(version == 1 && features.reasoningEffort && features.explicitGenerationReasoning && features.rejoin && templates.map { it.id }.distinct().size == templates.size)
        templates.forEach { role ->
            require(runCatching { java.util.UUID.fromString(role.id).toString().equals(role.id, true) }.getOrDefault(false))
            require(role.roleId.isNotBlank() && role.name.isNotEmpty() && role.description.isNotEmpty() && role.prompt.isNotBlank())
            require(role.name.values.all { it.isNotBlank() } && role.description.values.all { it.isNotBlank() })
            require(role.toolPack.id.isNotBlank() && role.toolPack.tools.all { it.isNotBlank() })
            require(role.skills.all { it.id.isNotBlank() && it.name.isNotEmpty() && it.name.values.all(String::isNotBlank) && it.instructions.isNotBlank() })
        }
        return this
    }
}
fun expertRoleText(value: Map<String, String>, languageTag: String): String =
    value[languageTag] ?: value[languageTag.substringBefore('-')] ?: value["en"] ?: value.values.firstOrNull().orEmpty()
