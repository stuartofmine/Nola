@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.ResourceAccessGrantRecord
import com.nola.domain.model.UploadedContent
import java.security.MessageDigest
import java.util.UUID
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

const val AGENT_CORE_RESOURCE_ACCESS_SCHEMA_VERSION = 1
const val AGENT_CORE_RESOURCE_ACCESS_ABI_VERSION = 1

@Serializable
enum class AgentCoreResourceKind {
    @SerialName("skill") Skill,
    @SerialName("mcp_server") McpServer,
    @SerialName("data_source") DataSource,
    @SerialName("prompt_template") PromptTemplate,
}

@Serializable
enum class AgentCoreResourcePermission {
    @SerialName("read_metadata") ReadMetadata,
    @SerialName("read_content") ReadContent,
    @SerialName("edit_content") EditContent,
    @SerialName("delete") Delete,
    @SerialName("manage_sharing") ManageSharing,
}

@Serializable
enum class AgentCoreResourcePrincipalKind {
    @SerialName("local_user") LocalUser,
    @SerialName("account") Account,
    @SerialName("device") Device,
    @SerialName("group") Group,
    @SerialName("anyone") Anyone,
}

@Serializable
data class AgentCoreResourcePrincipal(
    val kind: AgentCoreResourcePrincipalKind,
    val id: String,
)

@Serializable
data class AgentCoreManagedResourceIdentity(
    val kind: AgentCoreResourceKind,
    @SerialName("resource_id") val resourceId: String,
    val owner: AgentCoreResourcePrincipal,
    @SerialName("host_id") val hostId: String? = null,
    val revision: Long,
    @SerialName("content_digest") val contentDigest: String? = null,
)

@Serializable
data class AgentCoreResourceGrant(
    @SerialName("grant_id") val grantId: String,
    @SerialName("resource_kind") val resourceKind: AgentCoreResourceKind,
    @SerialName("resource_id") val resourceId: String,
    val principal: AgentCoreResourcePrincipal,
    val permissions: Set<AgentCoreResourcePermission>,
    @SerialName("host_id") val hostId: String? = null,
    @SerialName("resource_revision") val resourceRevision: Long? = null,
    @SerialName("content_digest") val contentDigest: String? = null,
    @SerialName("expires_at_epoch_ms") val expiresAtEpochMs: Long? = null,
)

@Serializable
data class AgentCoreResourceAccessRequest(
    @SerialName("schema_version") val schemaVersion: Int = AGENT_CORE_RESOURCE_ACCESS_SCHEMA_VERSION,
    val resource: AgentCoreManagedResourceIdentity,
    val actor: AgentCoreResourcePrincipal,
    val permission: AgentCoreResourcePermission,
    val grants: List<AgentCoreResourceGrant> = emptyList(),
    @SerialName("now_epoch_ms") val nowEpochMs: Long = System.currentTimeMillis(),
    @SerialName("request_host_id") val requestHostId: String? = null,
)

@Serializable
data class AgentCoreResourceAccessDecision(
    val allowed: Boolean,
    val reason: String,
    @SerialName("tool_execution_authorized") val toolExecutionAuthorized: Boolean,
)

@Serializable
private data class AgentCoreResourceAccessResponse(
    @SerialName("schema_version") val schemaVersion: Int,
    val decision: AgentCoreResourceAccessDecision? = null,
)

object AgentCoreResourceAccess {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
    }

    val isAvailable: Boolean
        get() = runCatching {
            AgentCoreNative.nativeResourceAccessAbiVersion() == AGENT_CORE_RESOURCE_ACCESS_ABI_VERSION
        }.getOrDefault(false)

    fun evaluate(request: AgentCoreResourceAccessRequest): AgentCoreResourceAccessDecision? {
        if (!isAvailable) return null
        val input = json.encodeToString(request).encodeToByteArray()
        val output = AgentCoreNative.nativeResourceAccessEvaluate(input) ?: return null
        val response = runCatching {
            json.decodeFromString(AgentCoreResourceAccessResponse.serializer(), output.decodeToString())
        }.getOrNull() ?: return null
        return response.decision?.takeIf {
            response.schemaVersion == AGENT_CORE_RESOURCE_ACCESS_SCHEMA_VERSION &&
                !it.toolExecutionAuthorized
        }
    }
}

data class AndroidManagedResourceAccessProjection(
    val canReadMetadata: Boolean,
    val canReadContent: Boolean,
    val canEditContent: Boolean,
    val canDelete: Boolean,
)

object AndroidManagedResourceAccess {
    fun nextRevision(current: Long): Long = when {
        current >= Long.MAX_VALUE -> Long.MAX_VALUE
        current < 0 -> 1
        else -> current + 1
    }

    fun project(skill: UploadedContent, principalId: String): AndroidManagedResourceAccessProjection =
        project(
            kind = AgentCoreResourceKind.Skill,
            resourceId = skill.resourceId ?: stableResourceId("skill", skill.name),
            ownerId = skill.accessOwnerId ?: principalId,
            revision = skill.accessRevision,
            content = skill.content,
            grants = skill.accessGrants,
            principalId = principalId,
        )

    fun project(server: McpServerConfig, principalId: String): AndroidManagedResourceAccessProjection =
        project(
            kind = AgentCoreResourceKind.McpServer,
            resourceId = server.resourceId ?: stableResourceId("mcp", server.serverId),
            ownerId = server.accessOwnerId ?: principalId,
            revision = server.accessRevision,
            content = "${server.name}\n${server.url}",
            grants = server.accessGrants,
            principalId = principalId,
        )

    fun canReadContent(skill: UploadedContent, principalId: String): Boolean =
        project(skill, principalId).canReadContent

    private fun project(
        kind: AgentCoreResourceKind,
        resourceId: String,
        ownerId: String,
        revision: Long,
        content: String,
        grants: List<ResourceAccessGrantRecord>,
        principalId: String,
    ): AndroidManagedResourceAccessProjection {
        val actor = AgentCoreResourcePrincipal(AgentCoreResourcePrincipalKind.LocalUser, principalId)
        val identity = AgentCoreManagedResourceIdentity(
            kind = kind,
            resourceId = resourceId,
            owner = AgentCoreResourcePrincipal(AgentCoreResourcePrincipalKind.LocalUser, ownerId),
            revision = revision.coerceAtLeast(1),
            contentDigest = "sha256:${sha256(content)}",
        )
        fun allows(permission: AgentCoreResourcePermission): Boolean =
            AgentCoreResourceAccess.evaluate(
                AgentCoreResourceAccessRequest(
                    resource = identity,
                    actor = actor,
                    permission = permission,
                    grants = grants.mapNotNull { it.toCoreGrant(kind, resourceId) },
                )
            )?.allowed == true
        return AndroidManagedResourceAccessProjection(
            canReadMetadata = allows(AgentCoreResourcePermission.ReadMetadata),
            canReadContent = allows(AgentCoreResourcePermission.ReadContent),
            canEditContent = allows(AgentCoreResourcePermission.EditContent),
            canDelete = allows(AgentCoreResourcePermission.Delete),
        )
    }

    private fun ResourceAccessGrantRecord.toCoreGrant(
        kind: AgentCoreResourceKind,
        resourceId: String,
    ): AgentCoreResourceGrant? {
        val mappedPrincipalKind = when (principalKind) {
            "local_user" -> AgentCoreResourcePrincipalKind.LocalUser
            "account" -> AgentCoreResourcePrincipalKind.Account
            "device" -> AgentCoreResourcePrincipalKind.Device
            "group" -> AgentCoreResourcePrincipalKind.Group
            "anyone" -> AgentCoreResourcePrincipalKind.Anyone
            else -> return null
        }
        val mappedPermissions = permissions.mapNotNull { value ->
            when (value) {
                "read_metadata" -> AgentCoreResourcePermission.ReadMetadata
                "read_content" -> AgentCoreResourcePermission.ReadContent
                "edit_content" -> AgentCoreResourcePermission.EditContent
                "delete" -> AgentCoreResourcePermission.Delete
                "manage_sharing" -> AgentCoreResourcePermission.ManageSharing
                else -> null
            }
        }.toSet()
        return AgentCoreResourceGrant(
            grantId = grantId,
            resourceKind = kind,
            resourceId = resourceId,
            principal = AgentCoreResourcePrincipal(mappedPrincipalKind, principalId),
            permissions = mappedPermissions,
            hostId = hostId,
            resourceRevision = resourceRevision,
            contentDigest = contentDigest,
            expiresAtEpochMs = expiresAtEpochMs,
        )
    }

    private fun stableResourceId(kind: String, value: String): String =
        UUID.nameUUIDFromBytes("$kind:$value".encodeToByteArray()).toString().lowercase()

    private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
        .digest(value.encodeToByteArray())
        .joinToString("") { byte -> "%02x".format(byte) }
}
