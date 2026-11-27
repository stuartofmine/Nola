// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.components

import android.os.Build
import android.view.View
import android.view.inspector.WindowInspector
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SheetState
import androidx.compose.material3.SheetValue
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.dp
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.R
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.ReadableExpertTemplateCatalog
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.data.repository.AgentDirectory
import com.nola.data.repository.ConversationProgress
import com.nola.data.repository.GoalControlObservation
import com.nola.data.repository.ExpertRoleTemplate
import com.nola.data.repository.PersistentAgent
import com.nola.domain.model.ContextBreakdown
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.ui.assertAboveBottomInset
import com.nola.ui.screen.chat.ChatContextScreen
import com.nola.ui.screen.chat.ChatMoreSettingsScreen
import com.nola.ui.screen.chat.ChatUiState
import com.nola.ui.screen.chat.ChatViewModel
import com.nola.ui.screen.chat.ContextDisplayMode
import com.nola.ui.screen.chat.ConversationProgressPanel
import com.nola.ui.screen.chat.ExpertManagementDataSource
import com.nola.ui.screen.chat.ExpertManagementPanel
import com.nola.ui.screen.models.ModelInferenceTestSheet
import com.nola.ui.screen.scheduled.ExecutionResultDetailSheet
import com.nola.ui.screen.scheduled.ScheduledActionEditorSheet
import com.nola.ui.theme.NolaTheme
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@OptIn(ExperimentalMaterial3Api::class)
@RunWith(AndroidJUnit4::class)
class BottomSheetPresentationContractUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun startingAnotherGoalDoesNotTargetTheCancelledGoal() {
        val prior = ConversationProgress(
            conversationId = "conversation", objective = "Previous goal", status = "cancelled",
            requirements = emptyList(), todos = emptyList(),
            goalControl = GoalControlObservation("old-goal", "old-token", 5uL, emptyMap(),
                canPause = false, canResume = false, canCancel = false)
        )
        var submitted: JsonObject? = null
        compose.setContent {
            NolaTheme {
                ConversationProgressPanel(
                    state = ChatUiState(conversationId = prior.conversationId),
                    load = { prior }, pendingControl = { false },
                    submitControl = { _, input -> submitted = input; prior }, onClose = {}
                )
            }
        }
        compose.onNodeWithText(compose.activity.getString(R.string.goal_objective))
            .performScrollTo().performTextReplacement("New objective")
        compose.onNodeWithText(compose.activity.getString(R.string.goal_start)).performScrollTo().performClick()
        compose.runOnIdle {
            assertEquals(JsonPrimitive("start"), submitted?.get("action"))
            assertEquals(JsonPrimitive("New objective"), submitted?.get("objective"))
            assertEquals(JsonNull, submitted?.get("expectedGoalId"))
            assertEquals(JsonNull, submitted?.get("expectedControlToken"))
        }
    }

    private enum class SheetCase {
        ExpertManagement,
        ChatMoreSettings,
        ChatContext,
        ConversationProgress,
        ModelParameters,
        ModelInferenceTest,
        ScheduledActionEditor,
        ExecutionResultDetail,
    }

    @Test
    fun allModalBottomSheetsOpenExpandedAndKeepBottomControlsAboveSimulatedNavigationInset() {
        var activeSheet by mutableStateOf<SheetCase?>(null)
        var simulatedBottomInsetPx = 0
        val sheetStates = mutableMapOf<SheetCase, SheetState>()

        val sampleModel = ModelInfo(
            id = "gpt-4.1",
            name = "GPT-4.1",
            source = ModelInfo.ModelSource.Cloud,
            profileId = "profile-1",
            profileRevision = 1L,
        )
        val sampleAction = ScheduledActionEntity(
            id = "action-1",
            localUserId = "local",
            name = "Daily Brief",
            url = "https://example.com/brief.json",
            format = "json",
            userPrompt = "Summarize highlights",
            scheduleFrequency = "daily",
            scheduledTimeIso = "09:00",
            timezone = "UTC",
        )
        val sampleResult = ExecutionResultEntity(
            id = "result-1",
            actionId = "action-1",
            sourceName = "Daily Brief",
            sourceUrl = "https://example.com/brief.json",
            sourceFormat = "json",
            status = "answered",
            llmSummaryText = "All systems nominal.",
        )

        compose.setContent {
            simulatedBottomInsetPx = with(LocalDensity.current) { 48.dp.roundToPx() }
            NolaTheme {
                when (activeSheet) {
                    SheetCase.ExpertManagement -> HostModalSheet(
                        onSheetState = { sheetStates[SheetCase.ExpertManagement] = it },
                    ) {
                        ExpertManagementPanel(
                            state = ChatUiState(
                                conversationId = "conv-1",
                                currentModel = sampleModel,
                                availableModels = listOf(sampleModel),
                            ),
                            dataSource = FakeExpertManagementDataSource(),
                            onBack = {},
                        )
                    }
                    SheetCase.ChatMoreSettings -> HostModalSheet(
                        onSheetState = { sheetStates[SheetCase.ChatMoreSettings] = it },
                    ) {
                        Box(Modifier.fillMaxWidth().fillMaxHeight(0.92f).navigationBarsPadding()) {
                            ChatMoreSettingsScreen(
                                selectedModel = sampleModel,
                                contextUsage = ContextUsage(
                                    totalTokens = 1200,
                                    maxTokens = 32000,
                                    usagePercent = 3.75f,
                                    breakdown = ContextBreakdown(),
                                ),
                                agentModeEnabled = false,
                                permissionMode = AndroidAgentPermissionMode.Cautious,
                                hybridDeviceToolsEnabled = false,
                                hybridDeviceCapabilities = emptySet(),
                                pairedHostIdentity = "",
                                autoCompactTriggerPercent = 80,
                                responseStyleOverride = null,
                                sessionMemory = "",
                                longTermMemory = "",
                                userPreferences = "",
                                draftParameters = ModelParameters(),
                                conversationId = "conv-1",
                                generationOverride = ConversationGenerationOverride(),
                                onAgentModeChange = {},
                                onPermissionModeChange = {},
                                onHybridDeviceToolsChange = {},
                                onHybridDeviceCapabilityChange = { _, _ -> },
                                onForgetHostPairing = {},
                                onAutoCompactPercentChange = {},
                                onResponseStyleChange = {},
                                onSessionMemoryChange = {},
                                onLongTermMemoryChange = {},
                                onUserPreferencesChange = {},
                                onSkillToggle = { _, _ -> },
                                onSaveParameters = {},
                                onSaveGenerationOverride = {},
                                onReloadParameters = { true },
                                onSaveMemoryProfile = {},
                                onOpenContextSettings = {},
                                onOpenSkillLibrary = {},
                                onBack = {},
                            )
                        }
                    }
                    SheetCase.ChatContext -> HostModalSheet(
                        onSheetState = { sheetStates[SheetCase.ChatContext] = it },
                    ) {
                        Box(Modifier.fillMaxWidth().fillMaxHeight(0.92f).navigationBarsPadding()) {
                            ChatContextScreen(
                                displayMode = ContextDisplayMode.Context,
                                conversationId = "conv-1",
                                conversationTitle = "Design Review",
                                contextUsage = ContextUsage(
                                    totalTokens = 1200,
                                    maxTokens = 32000,
                                    usagePercent = 3.75f,
                                    breakdown = ContextBreakdown(),
                                ),
                                sessionMemory = "",
                                knowledgePoints = "",
                                conversationWiki = "",
                                longTermMemory = "",
                                userPreferences = "",
                                mcpServers = emptyList(),
                                uploadedSkills = emptyList(),
                                onSessionMemoryChange = {},
                                onConversationKnowledgeChange = { _, _ -> },
                                onLongTermMemoryChange = {},
                                onUserPreferencesChange = {},
                                onSkillToggle = { _, _ -> },
                                onSaveSkill = { _, _, _, _ -> },
                                onSkillDelete = {},
                                onDocumentAdd = {},
                                onDocumentDelete = {},
                                onSaveMemoryProfile = {},
                                onCompact = {},
                                onReload = {},
                                onSearchMemory = { emptyList() },
                                onBack = {},
                            )
                        }
                    }
                    SheetCase.ConversationProgress -> HostModalSheet(
                        onSheetState = { sheetStates[SheetCase.ConversationProgress] = it },
                    ) {
                        Box(Modifier.fillMaxWidth().fillMaxHeight(0.92f).navigationBarsPadding()) {
                            ConversationProgressPanel(
                                state = ChatUiState(conversationId = "conv-1", currentModel = sampleModel),
                                load = {
                                    ConversationProgress(
                                        conversationId = "conv-1",
                                        objective = null,
                                        status = null,
                                        requirements = emptyList(),
                                        todos = emptyList(),
                                    )
                                },
                                pendingControl = { false },
                                submitControl = { _, _ ->
                                    ConversationProgress(
                                        conversationId = "conv-1",
                                        objective = null,
                                        status = null,
                                        requirements = emptyList(),
                                        todos = emptyList(),
                                    )
                                },
                                onClose = {},
                            )
                        }
                    }
                    SheetCase.ModelParameters -> ModelParametersSheet(
                        modelName = sampleModel.name,
                        supportsTopK = false,
                        parameters = ModelParameters(),
                        onUpdate = {},
                        onDismiss = {},
                    )
                    SheetCase.ModelInferenceTest -> ModelInferenceTestSheet(
                        model = sampleModel,
                        prompt = "Hello",
                        output = "Streamed response",
                        running = false,
                        firstTokenMs = 120L,
                        totalMs = 450L,
                        error = null,
                        onPromptChange = {},
                        onRun = {},
                        onStop = {},
                        onDismiss = {},
                    )
                    SheetCase.ScheduledActionEditor -> ScheduledActionEditorSheet(
                        existing = sampleAction,
                        onSave = {},
                        onDismiss = {},
                    )
                    SheetCase.ExecutionResultDetail -> ExecutionResultDetailSheet(
                        result = sampleResult,
                        onDismiss = {},
                    )
                    null -> Unit
                }
            }
        }

        for (sheetCase in SheetCase.entries) {
            compose.runOnIdle { activeSheet = sheetCase }
            compose.waitUntil(5_000) {
                compose.onAllNodes(isDialog()).fetchSemanticsNodes().isNotEmpty()
            }
            compose.waitForIdle()
            injectBottomNavigationInsetToDialogWindows(simulatedBottomInsetPx)
            compose.waitForIdle()

            val dialogNode = compose.onNode(isDialog()).fetchSemanticsNode()
            val dialogBottom = dialogNode.boundsInRoot.bottom
            assertNull(
                "$sheetCase must skip partially expanded and not expose Expand action",
                dialogNode.config.getOrNull(SemanticsActions.Expand),
            )
            sheetStates[sheetCase]?.let { state ->
                assertEquals("$sheetCase must open directly in SheetValue.Expanded", SheetValue.Expanded, state.currentValue)
            }
            assertTrue("Simulated bottom inset must be positive", simulatedBottomInsetPx > 0)

            when (sheetCase) {
                SheetCase.ExpertManagement -> {
                    compose.waitUntil(5_000) {
                        compose.onAllNodesWithTag("expert.row.agent-1").fetchSemanticsNodes().isNotEmpty()
                    }
                    compose.onNodeWithTag("expert.row.agent-1")
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ExpertManagement row")
                }
                SheetCase.ChatMoreSettings -> {
                    compose.onNodeWithText(compose.activity.getString(R.string.personal_intelligence_title))
                        .performScrollTo()
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ChatMoreSettings bottom row")
                }
                SheetCase.ChatContext -> {
                    compose.onNodeWithText(compose.activity.getString(R.string.chat_run_compact))
                        .performScrollTo()
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ChatContext compact button")
                }
                SheetCase.ConversationProgress -> {
                    compose.onNodeWithText(compose.activity.getString(R.string.goal_start))
                        .performScrollTo()
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ConversationProgress start button")
                }
                SheetCase.ModelParameters -> {
                    compose.onNodeWithTag("model-parameters-save")
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ModelParameters save button")
                }
                SheetCase.ModelInferenceTest -> {
                    compose.onNodeWithText(compose.activity.getString(R.string.models_inference_test_run))
                        .performScrollTo()
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ModelInferenceTest run button")
                }
                SheetCase.ScheduledActionEditor -> {
                    val cancelLabel = compose.activity.getString(R.string.common_cancel)
                    compose.onNode(hasScrollToNodeAction()).performScrollToNode(hasText(cancelLabel))
                    compose.onNodeWithText(cancelLabel)
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ScheduledActionEditor cancel button")
                }
                SheetCase.ExecutionResultDetail -> {
                    compose.onNodeWithText(compose.activity.getString(R.string.common_close))
                        .performScrollTo()
                        .assertAboveBottomInset(dialogBottom, simulatedBottomInsetPx.toFloat(), "ExecutionResultDetail close button")
                }
            }

            compose.runOnIdle { activeSheet = null }
            compose.waitUntil(5_000) {
                compose.onAllNodes(isDialog()).fetchSemanticsNodes().isEmpty()
            }
        }
    }

    @Composable
    private fun HostModalSheet(
        onSheetState: (SheetState) -> Unit,
        content: @Composable () -> Unit,
    ) {
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        onSheetState(sheetState)
        ModalBottomSheet(
            onDismissRequest = {},
            sheetState = sheetState,
        ) {
            content()
        }
    }

    private fun injectBottomNavigationInsetToDialogWindows(simulatedBottomInsetPx: Int) {
        compose.runOnIdle {
            val activityDecor = compose.activity.window.decorView
            val allWindows = allWindowRootViews()
            allWindows.filter { it !== activityDecor }.forEach { dialogDecor ->
                dialogDecor.setOnApplyWindowInsetsListener { target, insets ->
                    val compat = WindowInsetsCompat.toWindowInsetsCompat(insets, target)
                    val currentNav = compat.getInsets(WindowInsetsCompat.Type.navigationBars())
                    val patched = WindowInsetsCompat.Builder(compat)
                        .setInsets(
                            WindowInsetsCompat.Type.navigationBars(),
                            Insets.of(
                                currentNav.left,
                                currentNav.top,
                                currentNav.right,
                                maxOf(currentNav.bottom, simulatedBottomInsetPx),
                            ),
                        )
                        .build()
                    target.onApplyWindowInsets(patched.toWindowInsets())
                }
                ViewCompat.requestApplyInsets(dialogDecor)
            }
        }
    }

    @Suppress("UNCHECKED_CAST")
    private fun allWindowRootViews(): List<View> {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            return WindowInspector.getGlobalWindowViews()
        }
        return runCatching {
            val wmgClass = Class.forName("android.view.WindowManagerGlobal")
            val instance = wmgClass.getMethod("getInstance").invoke(null)
            val viewsField = wmgClass.getDeclaredField("mViews").apply { isAccessible = true }
            (viewsField.get(instance) as? List<View>).orEmpty()
        }.getOrDefault(emptyList())
    }

    @Test fun hostExpertChoosesExecutorVariantAndReopensSavedDefinition() {
        val fixture = FakeExpertManagementDataSource(hostExecutor = true)
        compose.setContent { NolaTheme { ExpertManagementPanel(ChatUiState(), fixture) {} } }
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Research Specialist").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("Research Specialist").performClick()
        compose.onNodeWithTag("expert.editor.executor").performScrollTo().performClick()
        compose.onNodeWithText("Antigravity").performClick()
        compose.onNodeWithTag("expert.editor.executor.model").performScrollTo().performClick()
        compose.onNodeWithText("Gemini · Low").performClick()
        compose.onNodeWithTag("expert.editor.save").performScrollTo().performClick()
        compose.waitUntil(5_000) { fixture.saved != null }
        assertEquals("""{"backend":"antigravity","model":"gemini-variant-low"}""", fixture.saved?.defaults?.get("expertExecutor").toString())
        compose.onNodeWithText("Research Specialist").performClick()
        compose.onNodeWithTag("expert.editor.executor.model").performScrollTo()
        compose.onNodeWithText("Gemini · Low", substring = true).assertExists()
        val instrumentation = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.waitForIdle(150, 2500)
        java.io.File(instrumentation.targetContext.filesDir, "expert-executor-variant.png").outputStream().use {
            instrumentation.uiAutomation.takeScreenshot().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    private class FakeExpertManagementDataSource(private val hostExecutor: Boolean = false) : ExpertManagementDataSource {
        @Volatile var saved: PersistentAgent? = null
        override suspend fun expertEditorScope() = ChatViewModel.ExpertEditorScope(
            remote = hostExecutor,
            executors = if (hostExecutor) listOf(com.nola.data.remote.dto.ExecutionBackendDto(
                id = "antigravity", displayName = "Antigravity", availability = "available",
                capabilities = com.nola.data.remote.dto.ExecutionBackendCapabilitiesDto(true, true, true, true, true, true),
                models = listOf(com.nola.data.remote.dto.ExecutionBackendModelDto("gemini-variant-low", "Gemini · Low")),
            )) else emptyList(),
            models = emptyList(),
            defaultProfileId = null,
            efforts = emptyMap(),
        )

        override suspend fun directory(conversationId: String?, storeEpoch: String?) = AgentDirectory(
            revision = 1L,
            agents = listOf(
                PersistentAgent(
                    id = "agent-1",
                    name = "Research Specialist",
                    revision = 1L,
                    availability = "active",
                    defaults = buildJsonObject {},
                    description = "Synthesizes documents and sources",
                ),
            ),
        )

        override suspend fun roleTemplates(conversationId: String?, storeEpoch: String?) =
            com.nola.data.repository.ExpertRoleCatalog(1, com.nola.data.repository.ExpertRoleFeatures(true, true, true, externalExpertExecutors = hostExecutor), emptyList())

        override suspend fun readableExpertTemplates() = ReadableExpertTemplateCatalog(
            revision = 1L,
            templates = emptyList(),
        )

        override suspend fun expertDefaultReasoningEfforts(): List<String> = emptyList()
        override suspend fun reasoningEfforts(model: ModelInfo): List<String> = emptyList()
        override suspend fun createAgent(
            conversationId: String?,
            storeEpoch: String?,
            name: String,
            defaults: JsonObject,
            snapshot: AgentDirectory,
            description: String,
            agentId: String,
        ): AgentDirectory = snapshot

        override suspend fun updateAgent(
            conversationId: String?,
            storeEpoch: String?,
            original: PersistentAgent,
            name: String,
            description: String,
            snapshot: AgentDirectory,
        ): AgentDirectory {
            if (!hostExecutor) return snapshot
            val updated = original.copy(name = name, description = description, revision = original.revision + 1)
            saved = updated
            return AgentDirectory((snapshot.revision ?: 0) + 1, snapshot.agents.map { if (it.id == updated.id) updated else it })
        }

        override fun refreshConversationExperts(restoreSelection: Boolean) = Unit
    }
}
