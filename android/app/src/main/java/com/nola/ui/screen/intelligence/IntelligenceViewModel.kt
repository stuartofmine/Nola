package com.nola.ui.screen.intelligence

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.R
import com.nola.core.AgentCoreManagedResourceIdentity
import com.nola.core.AgentCoreResourceAccess
import com.nola.core.AgentCoreResourceAccessRequest
import com.nola.core.AgentCoreResourceGrant
import com.nola.core.AgentCoreResourceKind
import com.nola.core.AgentCoreResourcePermission
import com.nola.core.AgentCoreResourcePrincipal
import com.nola.core.AgentCoreResourcePrincipalKind
import com.nola.core.AndroidManagedResourceAccess
import com.nola.core.AndroidManagedResourceAccessProjection
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.serialization.json.jsonPrimitive
import com.nola.domain.model.ChatMemoryProfile
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.ResourceAccessGrantRecord
import com.nola.domain.model.UploadedContent
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import java.security.MessageDigest
import java.util.UUID
import javax.inject.Inject
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

enum class IntelligenceEntryKind {
    Preference,
    Memory
}

data class IntelligenceUiState(
    val profile: ChatMemoryProfile = ChatMemoryProfile(),
    val isSaving: Boolean = false,
    val isLoadingProfile: Boolean = true,
    val profileLoadError: String? = null,
    val errorMessage: String? = null,
    val saveRevision: Long = 0,
    val skillAccess: List<AndroidManagedResourceAccessProjection> = emptyList(),
    val mcpAccess: List<AndroidManagedResourceAccessProjection> = emptyList(),
)

@HiltViewModel
class IntelligenceViewModel @Inject constructor(
    private val memoryProfileDataStore: MemoryProfileDataStore,
    @ApplicationContext private val context: Context,
) : ViewModel() {
    private val _uiState = MutableStateFlow(IntelligenceUiState())
    val uiState = _uiState.asStateFlow()
    private val saveMutex = Mutex()
    private var profileJob: kotlinx.coroutines.Job? = null

    init {
        reloadProfile()
    }

    fun reloadProfile() {
        profileJob?.cancel()
        _uiState.update { it.copy(isLoadingProfile = true, profileLoadError = null) }
        profileJob = viewModelScope.launch {
            try {
            memoryProfileDataStore.profile.collect { profile ->
                val principalId = memoryProfileDataStore.getOrCreateResourcePrincipalId()
                _uiState.update {
                    it.copy(
                        profile = profile,
                        isLoadingProfile = false,
                        profileLoadError = null,
                        skillAccess = profile.uploadedSkills.map { skill ->
                            AndroidManagedResourceAccess.project(skill, principalId)
                        },
                        mcpAccess = profile.mcpServers.map { server ->
                            AndroidManagedResourceAccess.project(server, principalId)
                        },
                    )
                }
            }
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update { it.copy(isLoadingProfile = false, profileLoadError = error.localizedMessage ?: error.javaClass.simpleName) }
            }
        }
    }

    fun saveEntry(kind: IntelligenceEntryKind, index: Int?, value: String) {
        val normalized = value.trim()
        if (normalized.isEmpty()) return
        mutateMemory { profile ->
            when (kind) {
                IntelligenceEntryKind.Preference -> profile.copy(
                    userPreferences = upsert(profile.userPreferences, index, normalized)
                )
                IntelligenceEntryKind.Memory -> profile.copy(
                    longTermMemory = upsert(profile.longTermMemory, index, normalized)
                )
            }
        }
    }

    fun deleteEntry(kind: IntelligenceEntryKind, index: Int) {
        mutateMemory { profile ->
            when (kind) {
                IntelligenceEntryKind.Preference -> profile.copy(
                    userPreferences = profile.userPreferences.filterIndexed { itemIndex, _ -> itemIndex != index }
                )
                IntelligenceEntryKind.Memory -> profile.copy(
                    longTermMemory = profile.longTermMemory.filterIndexed { itemIndex, _ -> itemIndex != index }
                )
            }
        }
    }

    fun openSkill(skill: UploadedContent, onLoaded: (UploadedContent, AndroidApplicationRuntime.SkillEditorSnapshot) -> Unit) {
        mutateMcp(recordSave = false) { principalId ->
            requireAccess(skill, principalId, AgentCoreResourcePermission.ReadContent)
            val snapshot = memoryProfileDataStore.skillEditor(requireNotNull(skill.resourceId))
            val record = snapshot.record
            onLoaded(skill.copy(
                name = record.getValue("display_name").jsonPrimitive.content,
                content = record.getValue("instructions_text").jsonPrimitive.content,
                enabled = record.getValue("enabled").jsonPrimitive.content.toBooleanStrict(),
            ), snapshot)
        }
    }

    fun saveSkill(original: UploadedContent, snapshot: AndroidApplicationRuntime.SkillEditorSnapshot?, name: String, content: String, onCommitted: () -> Unit) {
        val normalizedName = name.trim()
        val normalizedContent = content.trim()
        if (normalizedName.isEmpty() || normalizedContent.isEmpty()) return
        mutateMcp { principalId ->
            if (snapshot != null) requireAccess(original, principalId, AgentCoreResourcePermission.EditContent)
            val skill = original.copy(
                name = normalizedName,
                content = normalizedContent,
            )
            memoryProfileDataStore.saveSkill(snapshot, skill)
            onCommitted()
        }
    }

    fun setSkillEnabled(index: Int, enabled: Boolean) {
        val skill = _uiState.value.profile.uploadedSkills.getOrNull(index) ?: return
        mutateMcp { principalId ->
            requireAccess(skill, principalId, AgentCoreResourcePermission.EditContent)
            memoryProfileDataStore.setSkillEnabled(
                memoryProfileDataStore.skillEditor(requireNotNull(skill.resourceId)), enabled
            )
        }
    }

    fun deleteSkill(skill: UploadedContent, snapshot: AndroidApplicationRuntime.SkillEditorSnapshot) {
        mutateMcp { principalId ->
            requireAccess(skill, principalId, AgentCoreResourcePermission.Delete)
            memoryProfileDataStore.deleteSkill(snapshot)
        }
    }

    fun saveMcpServer(original: McpServerConfig?, name: String, url: String, onCommitted: () -> Unit = {}) {
        val normalizedName = name.trim()
        val normalizedUrl = url.trim()
        if (normalizedName.isEmpty() || normalizedUrl.isEmpty()) return
        mutateMcp { principalId ->
            if (original != null) requireAccess(original, principalId, AgentCoreResourcePermission.EditContent)
            val edited = (original ?: McpServerConfig(name = normalizedName, url = normalizedUrl)).copy(
                name = normalizedName,
                url = normalizedUrl,
                resourceId = original?.resourceId ?: UUID.randomUUID().toString().lowercase(),
                accessOwnerId = original?.accessOwnerId ?: principalId,
                accessRevision = AndroidManagedResourceAccess.nextRevision(original?.accessRevision ?: 0),
            )
            memoryProfileDataStore.saveMcpServer(original, edited)
            onCommitted()
        }
    }

    fun setMcpEnabled(index: Int, enabled: Boolean) {
        val original = _uiState.value.profile.mcpServers.getOrNull(index) ?: return
        mutateMcp { principalId ->
            requireAccess(original, principalId, AgentCoreResourcePermission.EditContent)
            memoryProfileDataStore.saveMcpServer(original, original.copy(
                enabled = enabled,
                accessRevision = AndroidManagedResourceAccess.nextRevision(original.accessRevision),
            ))
        }
    }

    fun deleteMcpServer(original: McpServerConfig) {
        mutateMcp { principalId ->
            requireAccess(original, principalId, AgentCoreResourcePermission.Delete)
            memoryProfileDataStore.deleteMcpServer(original)
        }
    }

    private fun mutateMcp(recordSave: Boolean = true, operation: suspend (String) -> Unit) {
        if (_uiState.value.isSaving || _uiState.value.isLoadingProfile || _uiState.value.profileLoadError != null) return
        _uiState.update { it.copy(isSaving = true, errorMessage = null) }
        viewModelScope.launch {
            saveMutex.withLock {
                _uiState.update { it.copy(isSaving = true, errorMessage = null) }
                try {
                    operation(memoryProfileDataStore.getOrCreateResourcePrincipalId())
                    if (recordSave) _uiState.update { it.copy(saveRevision = it.saveRevision + 1) }
                } catch (error: Exception) {
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    _uiState.update { it.copy(errorMessage = error.localizedMessage ?: error.javaClass.simpleName) }
                } finally {
                    _uiState.update { it.copy(isSaving = false) }
                }
            }
        }
    }

    fun clearError() {
        _uiState.update { it.copy(errorMessage = null) }
    }

    private fun mutateMemory(transform: (ChatMemoryProfile) -> ChatMemoryProfile) {
        val state = _uiState.value
        if (state.isSaving || state.isLoadingProfile || state.profileLoadError != null) return
        val draft = transform(state.profile)
        _uiState.update { it.copy(isSaving = true, errorMessage = null) }
        viewModelScope.launch {
            saveMutex.withLock {
                try {
                    memoryProfileDataStore.saveMemoryText(draft)
                    _uiState.update { state ->
                        state.copy(saveRevision = state.saveRevision + 1)
                    }
                } catch (error: Exception) {
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    _uiState.update { it.copy(errorMessage = error.localizedMessage ?: error.javaClass.simpleName) }
                } finally {
                    _uiState.update { it.copy(isSaving = false) }
                }
            }
        }
    }

    private fun upsert(items: List<String>, index: Int?, value: String): List<String> {
        if (index == null || index !in items.indices) return items + value
        return items.mapIndexed { itemIndex, item -> if (itemIndex == index) value else item }
    }

    private fun <T> upsertItem(items: List<T>, index: Int?, value: T): List<T> {
        if (index == null || index !in items.indices) return items + value
        return items.mapIndexed { itemIndex, item -> if (itemIndex == index) value else item }
    }

    private fun requireAccess(
        skill: UploadedContent,
        principalId: String,
        permission: AgentCoreResourcePermission,
    ) {
        requireAccess(
            kind = AgentCoreResourceKind.Skill,
            resourceId = skill.resourceId ?: stableResourceId("skill", skill.name),
            ownerId = skill.accessOwnerId ?: principalId,
            revision = skill.accessRevision,
            content = skill.content,
            grants = skill.accessGrants,
            principalId = principalId,
            permission = permission,
        )
    }

    private fun requireAccess(
        server: McpServerConfig,
        principalId: String,
        permission: AgentCoreResourcePermission,
    ) {
        requireAccess(
            kind = AgentCoreResourceKind.McpServer,
            resourceId = server.resourceId ?: stableResourceId("mcp", server.serverId),
            ownerId = server.accessOwnerId ?: principalId,
            revision = server.accessRevision,
            content = "${server.name}\n${server.url}",
            grants = server.accessGrants,
            principalId = principalId,
            permission = permission,
        )
    }

    private fun requireAccess(
        kind: AgentCoreResourceKind,
        resourceId: String,
        ownerId: String,
        revision: Long,
        content: String,
        grants: List<ResourceAccessGrantRecord>,
        principalId: String,
        permission: AgentCoreResourcePermission,
    ) {
        val actor = AgentCoreResourcePrincipal(AgentCoreResourcePrincipalKind.LocalUser, principalId)
        val identity = AgentCoreManagedResourceIdentity(
            kind = kind,
            resourceId = resourceId,
            owner = AgentCoreResourcePrincipal(AgentCoreResourcePrincipalKind.LocalUser, ownerId),
            revision = revision.coerceAtLeast(1),
            contentDigest = "sha256:${sha256(content)}",
        )
        val decision = AgentCoreResourceAccess.evaluate(
            AgentCoreResourceAccessRequest(
                resource = identity,
                actor = actor,
                permission = permission,
                grants = grants.mapNotNull { it.toCoreGrant(kind, resourceId) },
            )
        )
        if (decision?.allowed != true) {
            throw SecurityException(context.getString(R.string.intelligence_resource_access_denied))
        }
    }

    private fun ResourceAccessGrantRecord.toCoreGrant(
        kind: AgentCoreResourceKind,
        resourceId: String,
    ): AgentCoreResourceGrant? {
        val principalKind = when (principalKind) {
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
            principal = AgentCoreResourcePrincipal(principalKind, principalId),
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
