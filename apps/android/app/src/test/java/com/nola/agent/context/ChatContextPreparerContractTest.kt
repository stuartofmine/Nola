// SPDX-License-Identifier: Apache-2.0

package com.nola.agent.context

import com.nola.core.AgentCoreContextComponent
import com.nola.core.AgentCoreContextComponentMetrics
import com.nola.core.AgentCoreContextComponentSource
import com.nola.core.AgentCoreContextMetrics
import com.nola.core.AgentCoreContextPrepareOutput
import com.nola.core.AgentCoreDataSourceResolution
import com.nola.core.AndroidContextRoute
import com.nola.core.AndroidPreparedContext
import com.nola.core.AndroidSystemPrompt
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.data.health.AndroidHealthSummary
import com.nola.data.repository.selectUsableDataSources
import com.nola.domain.model.ChatRole
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.DataSourceReadMethod
import com.nola.domain.model.DataSourceReadRecipe
import com.nola.domain.model.DataSourceValidationContract
import com.nola.domain.model.ModelInput
import com.nola.domain.model.UploadedContent
import java.time.Instant
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatContextPreparerContractTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun extractsAttachmentComponentWithTitleAndStripsMarkupFromMessage() {
        val message = ConversationMessage(
            id = "m-1",
            conversationId = "conv-1",
            role = ChatRole.user,
            text = """Please review <attachment name="report.pdf">PDF text body</attachment> carefully.""",
        )

        val payload = extractConversationPayload(listOf(message))

        assertEquals(1, payload.attachments.size)
        val attachment = payload.attachments.single()
        assertEquals("attachment", attachment.kind)
        assertEquals("report.pdf", attachment.source?.title)
        assertEquals("PDF text body", attachment.content.jsonPrimitive.content)
        val renderedMessage = payload.messages.single()["content"]!!.jsonPrimitive.content
        assertFalse(renderedMessage.contains("<attachment"))
        assertTrue(renderedMessage.contains("Please review"))
        assertTrue(renderedMessage.contains("carefully."))
    }

    @Test
    fun conversationKnowledgeAndWikiUseCanonicalDocumentKind() {
        val conversation = Conversation(
            id = "conv-1",
            knowledgePoints = "Release deadline is Friday.",
            wiki = "Project wiki notes",
        )

        val components = assembleChatContextComponents(
            systemPrompt = null,
            builtComponents = emptyList(),
            conversation = conversation,
            knowledgeTitle = "Conversation Knowledge",
            wikiTitle = "Conversation Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health Connect",
            uploadedSkills = emptyList(),
            uploadedDocuments = emptyList(),
            attachments = emptyList(),
        )

        assertEquals(2, components.size)
        assertTrue(components.all { it.kind == "document" })
        assertEquals("conversation-knowledge-conv-1", components[0].id)
        assertEquals("Conversation Knowledge", components[0].source?.title)
        assertEquals("conversation-wiki-conv-1", components[1].id)
        assertEquals("Conversation Wiki", components[1].source?.title)
    }

    @Test
    fun uploadedSkillsAndDocumentsDeduplicateAndFilterDisabledOrBlankEntries() {
        val builtComponents = listOf(
            AgentCoreContextComponent(
                kind = "skill",
                id = "skill:existing-id",
                content = JsonPrimitive("{}"),
                source = AgentCoreContextComponentSource(platform = "android", title = "Existing By Title"),
            ),
            AgentCoreContextComponent(
                kind = "document",
                id = "doc:1",
                content = JsonPrimitive("Already included"),
                source = AgentCoreContextComponentSource(platform = "android", title = "Shared Handbook"),
            ),
        )
        val uploadedSkills = listOf(
            UploadedContent(
                name = "Disabled Skill",
                content = "Never inject disabled skills",
                enabled = false,
                resourceId = "disabled-skill-id",
            ),
            UploadedContent(
                name = "   ",
                content = "Never inject blank-named skills",
                enabled = true,
                resourceId = "blank-name-id",
            ),
            UploadedContent(
                name = "Brand New Title Same ID",
                content = "Should be deduped by skill id",
                description = "Skip",
                resourceId = "existing-id",
            ),
            UploadedContent(
                name = "Existing By Title",
                content = "Should be deduped by source title",
                description = "Skip",
                resourceId = "different-id",
            ),
            UploadedContent(
                name = "deploy-helper",
                content = "# Heading\nVerify release checksums before upload.\nSecond line.",
                description = "",
                resourceId = "deploy-helper-id",
            ),
            UploadedContent(
                name = "title-fallback-skill",
                content = "# Only headings",
                description = "",
                resourceId = null,
            ),
        )
        val uploadedDocuments = listOf(
            UploadedContent(name = "Disabled Doc", content = "Hidden", enabled = false),
            UploadedContent(name = "Empty Doc", content = "   ", enabled = true),
            UploadedContent(name = "Shared Handbook", content = "Duplicate document"),
            UploadedContent(name = "Runbook", content = "Unique document body"),
        )

        val components = assembleChatContextComponents(
            systemPrompt = "System prompt",
            builtComponents = builtComponents,
            conversation = Conversation(id = "conv-1"),
            knowledgeTitle = "Knowledge",
            wikiTitle = "Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health",
            uploadedSkills = uploadedSkills,
            uploadedDocuments = uploadedDocuments,
            attachments = emptyList(),
        )

        val addedSkills = components.filter { it.kind == "skill" && it.id != "skill:existing-id" }
        assertEquals(2, addedSkills.size)
        assertTrue(addedSkills.none { it.id == "skill:disabled-skill-id" || it.id == "skill:blank-name-id" })

        val deployMeta = json.parseToJsonElement(addedSkills[0].content.jsonPrimitive.content).jsonObject
        assertEquals("Verify release checksums before upload.", deployMeta["summary"]?.jsonPrimitive?.content)
        assertTrue(deployMeta["usage_contract"]?.jsonPrimitive?.content?.contains("skill_invoke") == true)

        val fallbackMeta = json.parseToJsonElement(addedSkills[1].content.jsonPrimitive.content).jsonObject
        assertEquals("title-fallback-skill", fallbackMeta["summary"]?.jsonPrimitive?.content)
        assertFalse(fallbackMeta["usage_contract"]?.jsonPrimitive?.content.isNullOrBlank())

        val addedDocuments = components.filter { it.kind == "document" && it.id != "doc:1" }
        assertEquals(1, addedDocuments.size)
        assertEquals("Runbook", addedDocuments.single().source?.title)

        val nonSystem = nonSystemTurnContextComponents(components)
        assertTrue(nonSystem.none { it.kind == "system" })
        assertTrue(nonSystem.any { it.kind == "skill" })
        assertTrue(nonSystem.any { it.kind == "document" })
    }

    @Test
    fun localVsCloudRoutesGateSystemPromptAndCloudContextDisclosure() {
        assertFalse(shouldIncludeSystemPrompt(AndroidContextRoute.Local))
        assertFalse(disclosesContextToCloud(AndroidContextRoute.Local))
        for (remoteRoute in listOf(
            AndroidContextRoute.Cloud,
            AndroidContextRoute.Web,
            AndroidContextRoute.PermissionResume,
            AndroidContextRoute.DeviceResume,
        )) {
            assertTrue("$remoteRoute must include system prompt", shouldIncludeSystemPrompt(remoteRoute))
            assertTrue("$remoteRoute discloses context outside device", disclosesContextToCloud(remoteRoute))
        }

        val fragments = AndroidSystemPrompt.agentFragments(
            stylePrompt = "Be concise.",
            customSystemPrompt = "Focus on facts.",
        )
        assertTrue(fragments.any { it.id == "android:host-identity" })
        assertTrue(fragments.any { it.id == "android:user-response-style" })
        assertTrue(fragments.any { it.id == "android:user-custom-instructions" })

        val localComponents = assembleChatContextComponents(
            systemPrompt = null,
            builtComponents = emptyList(),
            conversation = Conversation(id = "conv-local", knowledgePoints = "Local fact"),
            knowledgeTitle = "Knowledge",
            wikiTitle = "Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health",
            uploadedSkills = emptyList(),
            uploadedDocuments = emptyList(),
            attachments = emptyList(),
        )
        assertNull(localComponents.firstOrNull { it.kind == "system" })

        val cloudComponents = assembleChatContextComponents(
            systemPrompt = "Composed Cloud system policy",
            builtComponents = emptyList(),
            conversation = Conversation(id = "conv-cloud", knowledgePoints = "Cloud fact"),
            knowledgeTitle = "Knowledge",
            wikiTitle = "Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health",
            uploadedSkills = emptyList(),
            uploadedDocuments = emptyList(),
            attachments = emptyList(),
        )
        val systemComponent = cloudComponents.single { it.kind == "system" }
        assertEquals("android-chat-system", systemComponent.id)
        assertEquals("Composed Cloud system policy", systemComponent.content.jsonPrimitive.content)

        val localOnlySource = sampleDataSource("ds-local", allowCloudContext = false)
        val cloudAllowedSource = sampleDataSource("ds-cloud", allowCloudContext = true)
        val alwaysUsable: (CustomDataSourceConfig, Set<String>) -> AgentCoreDataSourceResolution = { _, _ ->
            AgentCoreDataSourceResolution(
                schemaVersion = 1,
                enabled = true,
                usable = true,
                shouldRequestConsent = false,
                shouldRefresh = false,
                reason = "ok",
            )
        }

        val localUsable = selectUsableDataSources(
            sources = listOf(localOnlySource, cloudAllowedSource),
            selectedIds = setOf("ds-local", "ds-cloud"),
            disclosesToCloud = disclosesContextToCloud(AndroidContextRoute.Local),
            resolve = alwaysUsable,
        )
        assertEquals(listOf("ds-local", "ds-cloud"), localUsable.map { it.id })

        val cloudUsable = selectUsableDataSources(
            sources = listOf(localOnlySource, cloudAllowedSource),
            selectedIds = setOf("ds-local", "ds-cloud"),
            disclosesToCloud = disclosesContextToCloud(AndroidContextRoute.Cloud),
            resolve = alwaysUsable,
        )
        assertEquals(listOf("ds-cloud"), cloudUsable.map { it.id })
    }

    @Test
    fun healthConnectPermissionStatesMapToCanonicalAuthorizations() {
        val observedAt = Instant.parse("2026-10-09T08:30:00Z")
        val cases = listOf(
            AndroidHealthSummary(
                availability = AndroidHealthAvailability.Available,
                permissionState = AndroidHealthPermissionState.Granted,
                totalSteps = 7_400,
                averageHeartRateBpm = 64,
                averageSleepHoursPerDay = 7.5,
                latestWorkoutType = "Running",
                latestWorkoutMinutes = 35,
                updatedAt = observedAt,
            ) to "granted",
            AndroidHealthSummary(
                availability = AndroidHealthAvailability.Available,
                permissionState = AndroidHealthPermissionState.Missing,
                missingPermissionCount = 4,
            ) to "not_determined",
            AndroidHealthSummary(
                availability = AndroidHealthAvailability.Unavailable,
                permissionState = AndroidHealthPermissionState.Unavailable,
            ) to "unavailable",
            AndroidHealthSummary(
                availability = AndroidHealthAvailability.ProviderUpdateRequired,
                permissionState = AndroidHealthPermissionState.Unavailable,
            ) to "unavailable",
            AndroidHealthSummary(
                availability = AndroidHealthAvailability.Available,
                permissionState = AndroidHealthPermissionState.Granted,
                error = "Health Connect read timeout",
            ) to "error",
        )

        cases.forEach { (summary, expectedAuthorization) ->
            val component = assembleChatContextComponents(
                systemPrompt = null,
                builtComponents = emptyList(),
                conversation = Conversation(id = "conv-health"),
                knowledgeTitle = "Knowledge",
                wikiTitle = "Wiki",
                personalContext = null,
                healthSummary = summary,
                healthTitle = "Health Connect",
                uploadedSkills = emptyList(),
                uploadedDocuments = emptyList(),
                attachments = emptyList(),
            ).single()

            assertEquals("platform_data", component.kind)
            assertEquals("platform-data-health", component.id)
            assertEquals("Health Connect", component.source?.title)
            assertEquals(expectedAuthorization, component.source?.authorization)
            assertEquals(180L, component.source?.maximumAgeMinutes)
        }

        val grantedBody = assembleChatContextComponents(
            systemPrompt = null,
            builtComponents = emptyList(),
            conversation = Conversation(id = "conv-health"),
            knowledgeTitle = "Knowledge",
            wikiTitle = "Wiki",
            personalContext = null,
            healthSummary = cases.first().first,
            healthTitle = "Health Connect",
            uploadedSkills = emptyList(),
            uploadedDocuments = emptyList(),
            attachments = emptyList(),
        ).single().content.jsonPrimitive.content
        assertTrue(grantedBody.contains("Steps: 7400"))
        assertTrue(grantedBody.contains("Latest heart rate: 64 bpm"))
        assertTrue(grantedBody.contains("Latest sleep: 7.5 hours"))
        assertTrue(grantedBody.contains("Latest workout: Running, 35 min"))
    }

    @Test
    fun responseBasisDerivesFileMemoryKnowledgeAndSkillsFromComponentKinds() {
        assertEquals(emptySet<String>(), deriveResponseBasis(emptyList()))
        assertEquals(
            setOf("file"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("attachment", "att-1", JsonPrimitive("body")))),
        )
        assertEquals(
            setOf("memory"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("personal_intelligence", "pi-1", JsonPrimitive("note")))),
        )
        assertEquals(
            setOf("memory"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("user_preference", "pref-1", JsonPrimitive("concise")))),
        )
        assertEquals(
            setOf("knowledge"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("platform_data", "ds-1", JsonPrimitive("steps")))),
        )
        assertEquals(
            setOf("knowledge"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("mcp", "mcp-1", JsonPrimitive("schema")))),
        )
        assertEquals(
            setOf("skills"),
            deriveResponseBasis(listOf(AgentCoreContextComponent("skill", "skill:1", JsonPrimitive("{}")))),
        )
        assertEquals(
            setOf("file", "memory", "knowledge", "skills"),
            deriveResponseBasis(
                listOf(
                    AgentCoreContextComponent("system", "sys", JsonPrimitive("prompt")),
                    AgentCoreContextComponent("attachment", "att", JsonPrimitive("pdf")),
                    AgentCoreContextComponent("memory", "mem", JsonPrimitive("fact")),
                    AgentCoreContextComponent("document", "doc", JsonPrimitive("wiki")),
                    AgentCoreContextComponent("skill", "sk", JsonPrimitive("{}")),
                )
            ),
        )
    }

    @Test
    fun usageRecorderMapsUsedDataSourceIdsAndConsumerDisclosureByRoute() {
        val sources = listOf(
            sampleDataSource("weather", allowCloudContext = true),
            sampleDataSource("notes", allowCloudContext = false),
        )
        val prepared = AndroidPreparedContext(
            route = AndroidContextRoute.Cloud,
            modelInput = ModelInput(canonicalMessages = emptyList()),
            output = AgentCoreContextPrepareOutput(
                schemaVersion = 1,
                action = "ready",
                messages = emptyList(),
                payloadDigest = "digest",
                metrics = AgentCoreContextMetrics(10, 0, 10, 8192, 1024, 0.12),
                componentMetrics = listOf(
                    AgentCoreContextComponentMetrics(
                        kind = "platform_data",
                        usedTokens = 40,
                        droppedTokens = 0,
                        truncatedTokens = 0,
                        usedIds = listOf("data_source:weather", "data_source:weather", "platform-data-health", "data_source:"),
                    ),
                    AgentCoreContextComponentMetrics(
                        kind = "document",
                        usedTokens = 20,
                        droppedTokens = 0,
                        truncatedTokens = 0,
                        usedIds = listOf("data_source:notes"),
                    ),
                ),
            ),
        )

        val localRecords = buildUsageRecordsForPrepared(prepared, sources, "conv-1", AndroidContextRoute.Local)
        assertEquals(1, localRecords.size)
        assertEquals("weather", localRecords.single().sourceId)
        assertEquals("Android local model", localRecords.single().consumer)
        assertFalse(localRecords.single().disclosedToCloud)
        assertEquals("conv-1", localRecords.single().conversationId)

        val cloudRecords = buildUsageRecordsForPrepared(prepared, sources, "conv-1", AndroidContextRoute.Cloud)
        assertEquals("Android cloud model", cloudRecords.single().consumer)
        assertTrue(cloudRecords.single().disclosedToCloud)

        val webRecords = buildUsageRecordsForPrepared(prepared, sources, "conv-1", AndroidContextRoute.Web)
        assertEquals("Android Web Host", webRecords.single().consumer)
        assertTrue(webRecords.single().disclosedToCloud)

        val resumeRecords = buildUsageRecordsForPrepared(prepared, sources, "conv-1", AndroidContextRoute.PermissionResume)
        assertEquals("Android resumed agent", resumeRecords.single().consumer)
        assertTrue(resumeRecords.single().disclosedToCloud)
    }

    private fun sampleDataSource(id: String, allowCloudContext: Boolean) = CustomDataSourceConfig(
        id = id,
        name = "Source $id",
        location = "https://example.test/$id.json",
        enabled = true,
        accessPolicy = DataSourceAccessPolicy.ConversationOptIn,
        allowCloudContext = allowCloudContext,
        cachedContent = """{"ok":true}""",
        validationContract = DataSourceValidationContract(
            sourceId = id,
            readerDescription = "HTTP GET -> JSON -> RawEvent",
            format = "JSON",
            readRecipe = DataSourceReadRecipe(
                method = DataSourceReadMethod.HttpGet,
                operationId = "android.http.get",
                configurationKeys = listOf("url"),
                outputSchemaJson = """{"type":"object"}""",
            ),
        ),
    )
}
