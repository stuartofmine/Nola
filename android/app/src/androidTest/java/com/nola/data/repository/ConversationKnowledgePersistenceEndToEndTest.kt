package com.nola.data.repository

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.AndroidModelConfiguration
import com.nola.data.local.CanonicalModelProfile
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.domain.model.CompactedMemoryEntry
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ModelInfo
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ConversationKnowledgePersistenceEndToEndTest {
    private lateinit var repository: ConversationRepositoryImpl
    private lateinit var canonicalFile: File

    @Test
    fun configuredProfileIdentityIsDistinctFromProviderModelName() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val runtime = AndroidApplicationRuntime(context, canonicalFile)
        try {
            runtime.upsertModelProfile(CanonicalModelProfile(
                id = "profile-canonical-17", displayName = "Research", provider = "openai-compatible",
                modelName = "wire-model-2027", runtime = "remote",
                sourceJson = """{"schemaVersion":1,"baseURL":"https://example.invalid/v1"}""",
                parametersJson = """{"maxCompletionTokens":2048}""",
                selected = true, enabled = true, revision = 0, createdAtMs = 1, updatedAtMs = 1,
            ))
            val option = AndroidModelConfiguration(runtime).configuredProfiles().single().toConfiguredModelInfo()
            assertEquals("wire-model-2027", option.id)
            assertEquals("configured:profile-canonical-17", ConversationGenerationOverride.modelKey(option))
            assertEquals(2_048, ModelParametersStore(runtime).loadParameters(option.parameterProfileId).maxCompletionTokens)
        } finally {
            runtime.close()
        }
    }

    @Test
    fun modelCatalogReplacementRejectsStaleMembershipRevision() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val runtime = AndroidApplicationRuntime(context, canonicalFile)
        try {
            val profile = CanonicalModelProfile(
                id = "stable-profile", displayName = "Stable", provider = "openai-compatible",
                modelName = "wire-model", runtime = "remote",
                sourceJson = """{"schemaVersion":1,"baseURL":"https://example.invalid/v1"}""",
                parametersJson = "{}", selected = true, enabled = true,
                revision = 0, createdAtMs = 1, updatedAtMs = 1,
            )
            runtime.upsertModelProfile(profile)
            val baseline = requireNotNull(runtime.modelProfile(profile.id))
            val stale = mapOf(baseline.id to baseline.revision)
            runtime.upsertModelProfile(baseline.copy(displayName = "Changed elsewhere", updatedAtMs = 2))
            val conflict = runCatching {
                runtime.replaceModelProfiles("openai-compatible", "remote", listOf(baseline), stale, 3)
            }.exceptionOrNull()
            assertTrue(conflict?.message.orEmpty().contains("revision_conflict"))
            assertEquals("Changed elsewhere", runtime.modelProfile(profile.id)?.displayName)
            AndroidModelConfiguration(runtime).save("https://example.invalid/v1", "wire-model")
            assertEquals("stable-profile", AndroidModelConfiguration(runtime).configuredProfiles().single().id)
        } finally {
            runtime.close()
        }
    }

    @Test
    fun sparseConfigurationPatchRejectsStaleRevisionAndPreservesCrossClientFields() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val otherClient = AndroidApplicationRuntime(context, canonicalFile)
        try {
            val conversation = repository.createConversation(title = "Shared")
            val baseline = requireNotNull(otherClient.loadConversation(conversation.id))
            val model = ModelInfo("fixture", "Fixture", ModelInfo.ModelSource.Cloud, profileId = "fixture-profile")
            repository.updateGenerationOverride(conversation.id, model,
                ConversationGenerationOverride(maxOutputTokens = 4_096))
            val stale = otherClient.patchConversationConfiguration(
                conversation.id, requireNotNull(baseline.configurationRevision), """{"permissionProfile":"allow_all"}""")
            assertTrue(stale is com.nola.data.local.ConfigurationCommitResult.Conflict)
            repository.updatePermissionMode(conversation.id, "cautious",
                requireNotNull(repository.getConversation(conversation.id)).configurationRevision,
                com.nola.data.local.PermissionConfigurationOwner.Inactive)
            val reloaded = requireNotNull(repository.getConversation(conversation.id))
            assertEquals("cautious", reloaded.permissionMode)
            assertEquals(4_096,
                reloaded.generationOverrides.getValue("configured:fixture-profile").maxOutputTokens)
        } finally {
            otherClient.close()
        }
    }

    @Before
    fun setUp() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        canonicalFile = File(context.cacheDir, "conversation-knowledge-${System.nanoTime()}.db")
        repository = AndroidApplicationRuntime(context, canonicalFile).let {
            ConversationRepositoryImpl(it, com.nola.data.local.AndroidAssistantJournal(it), com.nola.llm.local.DeviceModelStore(context))
        }
    }

    @After
    fun tearDown() {
        canonicalFile.delete()
    }

    @Test
    fun promptStyleIsConversationScopedAndSurvivesColdReopen() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val runtime = AndroidApplicationRuntime(context, canonicalFile)
        val model = com.nola.data.local.CanonicalModelProfile(
            id = "style-fixture", displayName = "Style", provider = "test", modelName = "style",
            runtime = "android", sourceJson = "{}", parametersJson = """{"responseStyle":"professional"}""",
            selected = false, enabled = true, revision = 0, createdAtMs = 1, updatedAtMs = 1, deletedAtMs = null,
        )
        runtime.upsertModelProfile(model)
        val savedModel = runtime.modelProfile(model.id)
        val first = repository.createConversation(title = "First")
        val second = repository.createConversation(title = "Second")
        assertNull(first.promptResponseStyle)
        assertNull(second.promptResponseStyle)
        repository.updateResponseStyle(first.id, "playful")
        repository.updateSessionMemory(first.id, "Keep style")
        val reopened = AndroidApplicationRuntime(context, canonicalFile).let { ConversationRepositoryImpl(it, com.nola.data.local.AndroidAssistantJournal(it), com.nola.llm.local.DeviceModelStore(context)) }
        assertEquals("playful", reopened.getConversation(first.id)?.promptResponseStyle)
        assertNull(reopened.getConversation(second.id)?.promptResponseStyle)
        reopened.updateResponseStyle(first.id, null)
        assertNull(repository.getConversation(first.id)?.promptResponseStyle)
        assertEquals(savedModel, runtime.modelProfile(model.id))
        runtime.close()
    }

    @Test
    fun knowledgeAndWikiSurviveRepositoryReload() = runBlocking {
        val conversation = repository.createConversation(title = "Project Atlas")

        repository.updateKnowledge(
            conversation.id,
            knowledgePoints = "- Decision: local-first\n- Owner: Lin",
            wiki = "# Atlas\n\n## Architecture\nNative clients share Agent Core."
        )

        val reloaded = requireNotNull(repository.getConversation(conversation.id))
        assertEquals("- Decision: local-first\n- Owner: Lin", reloaded.knowledgePoints)
        assertEquals("# Atlas\n\n## Architecture\nNative clients share Agent Core.", reloaded.wiki)
    }

    @Test
    fun canonicalCompactionRejectsFabricatedLineageWithoutPublishingSummary() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val conversation = repository.createConversation(title = "Long session")
        val runtime = AndroidApplicationRuntime(context, canonicalFile)
        try {
            val source = runtime.prepareContextCompaction(conversation.id)
            val input = kotlinx.serialization.json.Json.parseToJsonElement("""{
                "schema_version":2,"mode":"force_summary", "messages":[
                    {"id":"fabricated-old","role":"user","content":"${"evidence ".repeat(1200)}"},
                    {"id":"fabricated-answer","role":"assistant","content":"${"observed ".repeat(1200)}"},
                    {"id":"latest","role":"user","content":"continue"}],
                "budget":{"prompt_token_limit":8000,"token_threshold":1000,"keep_recent_messages":1,"min_messages_before_compaction":0}
            }""")
            val command = kotlinx.serialization.json.Json.parseToJsonElement("""{
                "type":"context", "command":{"type":"commit_maintenance","input":{
                    "source":$source,"validated_source":$source,"summary":"uncommitted",
                    "original_input":$input,"current_input":$input,"committed_at_ms":1778000000000
                }}
            }""").let { it as kotlinx.serialization.json.JsonObject }
            assertTrue(runCatching { runtime.execute(command) }.isFailure)
            val restored = requireNotNull(repository.getConversation(conversation.id))
            assertEquals("", restored.sessionMemory)
            assertTrue(restored.compactedMemoryEntries.isEmpty())
        } finally { runtime.close() }
    }
}
