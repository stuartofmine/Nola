// SPDX-License-Identifier: Apache-2.0
package com.nola

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.graphics.Bitmap
import android.os.Build
import android.os.SystemClock
import android.view.WindowManager
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.isSelected
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.core.AgentCoreNative
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.CanonicalModelProfile
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.data.repository.toConfiguredModelInfo
import com.nola.domain.model.StructuredMemoryRecord
import com.nola.ui.components.StreamingMarkdownPresentedTextKey
import com.nola.ui.screen.chat.CHAT_STREAMING_MARKDOWN_TEST_TAG
import com.nola.ui.screen.chat.ChatComposerGenerationActiveKey
import com.nola.ui.screen.chat.ChatStreamingMarkdownSourceKey
import com.nola.ui.screen.chat.HistoricalPartRuntimeEntryPoint
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX
import dagger.hilt.android.EntryPointAccessors
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Lean, real-device / AVD instrumented E2E suite:
 * 1. `testRealModelChatAndAllScreens`: drives the real `MainActivity` Compose UI across Models,
 *    Chat (with a live LAN LLM reply), Activity, Information, Settings, and Capabilities while
 *    capturing compositor PNG screenshots under `files/e2e-screenshots/`.
 * 2. `testCoreStoreAndUnsupportedDbRejection`: verifies the real native Rust JNI store rejects
 *    legacy v1 and corrupt SQLite databases untouched and persists v2 conversations/models/memory.
 */
@RunWith(AndroidJUnit4::class)
class RealModelE2EAndroidTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val context: Context get() = instrumentation.targetContext
    private val primaryNavigation: SemanticsMatcher
        get() = hasTestTag("navigation.bottom") or hasTestTag("navigation.side.compact") or hasTestTag("navigation.side.expanded")

    private data class RealModelFixture(
        val endpoint: String,
        val model: String,
        val apiKey: String,
        val protocol: String,
    )

    @Test
    fun testRealModelChatAndAllScreens() {
        val fixture = loadRealModelFixture()
        assumeTrue(
            "Skipping real-model E2E: provide realLlmEndpoint/realLlmModel/realLlmApiKey instrumentation args or /data/local/tmp/nola-real-llm-e2e.json",
            fixture != null,
        )
        val activeFixture = requireNotNull(fixture)
        requireIsolatedAvdOrTestUser()

        composeRule.activityRule.scenario.onActivity { activity ->
            activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }

        val appCtx = context.applicationContext
        val runtime = EntryPointAccessors.fromApplication(appCtx, HistoricalPartRuntimeEntryPoint::class.java).applicationRuntime()
        val secureTokens = SecureTokenStore(appCtx)
        val preferences = PreferenceDataStore(appCtx, secureTokens)
        val memoryProfile = MemoryProfileDataStore(appCtx, secureTokens, runtime)
        runBlocking {
            preferences.setOnboardingCompleted()
            configureRealLanModel(runtime, secureTokens, memoryProfile, preferences, activeFixture)
        }
        waitForWorkspaceReady()

        // 1. Models screen: confirm real LAN model is visible & active -> android-01-models.png
        selectTab(R.string.tab_settings)
        composeRule.onNodeWithText(context.getString(R.string.models_title)).performProductScrollTo().performProductClick()
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodesWithTag("models.refresh", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        val modelTag = MODEL_ROW_TEST_TAG_PREFIX + activeFixture.model
        if (composeRule.onAllNodesWithTag(modelTag, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            composeRule.onNodeWithTag("models.refresh").performProductClick()
        }
        composeRule.waitUntil(20_000) {
            composeRule.onAllNodesWithTag(modelTag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        confirmFixtureCapabilitiesIfPresent(activeFixture.model)
        val useMatcher = hasText(context.getString(R.string.models_use)) and hasAnyAncestor(hasTestTag(modelTag))
        if (composeRule.onAllNodes(useMatcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()) {
            composeRule.onAllNodes(useMatcher, useUnmergedTree = true)[0].performProductScrollTo().performProductClick()
        }
        val selectedMatcher = hasText(context.getString(R.string.models_selected)) and hasAnyAncestor(hasTestTag(modelTag))
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodes(selectedMatcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        captureScreenshot("android-01-models.png")

        // 2. Chat tab: new conversation + real LAN model reply containing OK_ANDROID_REAL_MODEL -> android-02-chat-real-reply.png
        selectTab(R.string.tab_chat)
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodesWithTag("chat.conversations.new").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("chat.conversations.new").performProductClick()
        composeRule.waitUntil(20_000) {
            composeRule.onAllNodes(hasTestTag("chat.composer.input") and isEnabled()).fetchSemanticsNodes().isNotEmpty()
        }

        val priorSessions = runtime.listConversations().map { it.id }.toSet()
        val expectedMarker = "OK_ANDROID_REAL_MODEL"
        val prompt = "请用一句简短的中文回复，并在结尾准确附上 $expectedMarker，不要调用任何工具。"
        composeRule.onNodeWithTag("chat.composer.input").performProductInput(prompt)
        val sendMatcher = hasContentDescription(context.getString(R.string.chat_send))
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodes(sendMatcher and isEnabled()).fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNode(sendMatcher).performProductClick()

        awaitRealReply(expectedMarker)
        assertTrue(
            "Unexpected error recovery banner after real-model reply",
            composeRule.onAllNodesWithText(context.getString(R.string.chat_unknown_check), useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
        )
        assertTrue(
            "Permission state must not stay stuck checking after real-model reply",
            composeRule.onAllNodesWithText(context.getString(R.string.chat_permission_state_checking), useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
        )
        captureScreenshot("android-02-chat-real-reply.png")

        val owner = runtime.listConversations().single { it.id !in priorSessions }.id
        val originalUser = requireNotNull(runtime.loadConversation(owner)).messages.single { it.role == "user" }
        val laterMarker = "ANDROID_LATER_TURN_429"
        composeRule.onNodeWithTag("chat.composer.input").performProductInput("Reply exactly with $laterMarker.")
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodes(sendMatcher and isEnabled()).fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNode(sendMatcher).performProductClick()
        awaitRealReply(laterMarker)
        assertEquals(4, requireNotNull(runtime.loadConversation(owner)).messages.size)
        val editOriginal = hasTestTag("chat.message.edit." + originalUser.id)
        composeRule.onNodeWithTag("chat-transcript-list").performTouchInput { swipeDown() }
        composeRule.waitForIdle()
        captureScreenshot("android-earlier-turn.png")
        composeRule.onNode(editOriginal).performProductClick()
        val editedMarker = "ANDROID_EDIT_RETURN_681"
        val editedPrompt = "Reply exactly with $editedMarker."
        val editor = hasTestTag("chat.message.editor.input") and isEnabled()
        composeRule.waitUntil(15_000) { composeRule.onAllNodes(editor).fetchSemanticsNodes().size == 1 }
        composeRule.onNode(editor).performTextReplacement(editedPrompt)
        composeRule.onNodeWithTag("chat.message.editor.submit").performProductClick()
        awaitRealReply(editedMarker)
        val revised = requireNotNull(runtime.loadConversation(owner)).messages
        assertEquals(listOf("user", "assistant"), revised.map { it.role })
        assertEquals(editedPrompt, revised.first().text)
        assertTrue(revised.last().text.contains(editedMarker))
        assertEquals(setOf(owner), runtime.listConversations().map { it.id }.toSet() - priorSessions)
        captureScreenshot("android-edited-reply.png")

        // Leave conversation detail if phone layout hides bottom navigation inside chat.
        if (composeRule.onAllNodes(primaryNavigation).fetchSemanticsNodes().isEmpty()) {
            composeRule.onNodeWithTag("chat.conversation.back").performProductClick()
            composeRule.waitUntil(15_000) {
                composeRule.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty()
            }
        }

        // 3. Activity tab -> android-03-activity.png
        selectTab(R.string.tab_activity)
        captureScreenshot("android-03-activity.png")

        // 4. Information / Library tab (Sources, Memory) -> android-04-information.png
        selectTab(R.string.tab_information)
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodes(hasTestTag("information.overview") or hasTestTag("data.workspace"))
                .fetchSemanticsNodes().isNotEmpty()
        }
        captureScreenshot("android-04-information.png")

        // 5. Settings root & Capabilities (Skills / MCP / Experts) -> android-05-settings.png, android-06-capabilities.png
        selectTab(R.string.tab_settings)
        val capabilitiesLabel = context.getString(R.string.settings_capabilities)
        if (composeRule.onAllNodesWithText(capabilitiesLabel, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) {
            val backButton = hasContentDescription(context.getString(R.string.common_back))
            if (composeRule.onAllNodes(backButton, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()) {
                composeRule.onAllNodes(backButton, useUnmergedTree = true)[0].performProductClick()
            } else {
                composeRule.activityRule.scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }
            }
        }
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodesWithText(capabilitiesLabel, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        captureScreenshot("android-05-settings.png")

        composeRule.onNodeWithText(capabilitiesLabel, useUnmergedTree = true).performProductScrollTo().performProductClick()
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodesWithTag("intelligence.capabilities.skills").fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNodeWithTag("intelligence.capabilities.skills").assertIsDisplayed().assertIsSelected()
        captureScreenshot("android-06-capabilities.png")
    }

    @Test
    fun testCoreStoreAndUnsupportedDbRejection() {
        assertTrue("Native Rust JNI application library must be loaded", AgentCoreNative.isApplicationAvailable)
        val sandbox = File(context.cacheDir, "nola-store-contract-${System.nanoTime()}").apply { mkdirs() }
        try {
            // 1. Legacy SQLite v1 database (user_version = 1) must be cleanly rejected and left untouched.
            val v1Db = File(sandbox, "legacy-v1.db")
            SQLiteDatabase.openOrCreateDatabase(v1Db, null).use { db ->
                db.execSQL("PRAGMA user_version = 1;")
                db.execSQL("CREATE TABLE legacy_marker (id INTEGER PRIMARY KEY, note TEXT NOT NULL);")
                db.execSQL("INSERT INTO legacy_marker (id, note) VALUES (1, 'do-not-mutate');")
            }
            val v1BytesBefore = v1Db.readBytes()
            assertEquals(0L, AgentCoreNative.nativeApplicationOpen(v1Db.absolutePath.encodeToByteArray()))
            assertTrue(
                "AndroidApplicationRuntime must reject legacy v1 SQLite databases",
                runCatching { AndroidApplicationRuntime(context, v1Db).close() }.isFailure,
            )
            assertTrue("Legacy v1 database file must not be deleted", v1Db.isFile)
            assertArrayEquals("Legacy v1 database bytes must remain untouched", v1BytesBefore, v1Db.readBytes())

            // 2. Corrupt non-SQLite database file must be rejected cleanly without deleting or rewriting it.
            val corruptDb = File(sandbox, "corrupt.db")
            val corruptBytes = "not-a-valid-sqlite-database-header".toByteArray(Charsets.UTF_8)
            corruptDb.writeBytes(corruptBytes)
            assertEquals(0L, AgentCoreNative.nativeApplicationOpen(corruptDb.absolutePath.encodeToByteArray()))
            assertTrue(
                "AndroidApplicationRuntime must reject corrupt database files",
                runCatching { AndroidApplicationRuntime(context, corruptDb).close() }.isFailure,
            )
            assertArrayEquals("Corrupt database bytes must remain untouched", corruptBytes, corruptDb.readBytes())

            // 3. Clean v2 database persists conversations, model profiles, and memories across cold reopen.
            val v2Db = File(sandbox, "v2-product.db")
            val now = System.currentTimeMillis()
            val convId = "conv-${UUID.randomUUID()}"
            val modelId = "model-${UUID.randomUUID()}"
            val memoryId = "mem-${UUID.randomUUID()}"

            AndroidApplicationRuntime(context, v2Db).use { runtime ->
                runtime.createConversation(convId, "Android v2 Roundtrip", now, now, null)
                runtime.upsertModelProfile(
                    CanonicalModelProfile(
                        id = modelId,
                        displayName = "LAN Roundtrip Model",
                        provider = "openai-compatible",
                        modelName = "lan-roundtrip-model",
                        runtime = "remote",
                        sourceJson = """{"baseURL":"http://127.0.0.1:8080/v1","wireProtocol":"chat_completions","credentialRequired":false,"capabilities":["chat"]}""",
                        parametersJson = "{}",
                        selected = true,
                        enabled = true,
                        revision = 0,
                        createdAtMs = now,
                        updatedAtMs = now,
                    ),
                )
                runtime.upsertMemory(
                    StructuredMemoryRecord(
                        id = memoryId,
                        owner = buildJsonObject { put("kind", "user") },
                        namespace = "personal",
                        scope = "global",
                        kind = "preference",
                        content = "Prefer concise Chinese summaries",
                        confidence = 0.9,
                        provenanceKind = "explicit_user",
                        sourceReference = "android-e2e-test",
                        observedAt = now,
                        createdAt = now,
                        updatedAt = now,
                    ),
                )
            }

            AndroidApplicationRuntime(context, v2Db).use { reopened ->
                val conversation = reopened.loadConversation(convId)
                assertNotNull("Conversation must survive cold SQLite reopen", conversation)
                assertEquals("Android v2 Roundtrip", conversation?.title)

                val profile = reopened.listModelProfiles().singleOrNull { it.id == modelId }
                assertNotNull("Model profile must survive cold SQLite reopen", profile)
                assertEquals("lan-roundtrip-model", profile?.modelName)
                assertTrue("Model profile enabled flag must persist", profile?.enabled == true)

                val memory = reopened.listMemories().singleOrNull { it.id == memoryId }
                assertNotNull("Structured memory must survive cold SQLite reopen", memory)
                assertEquals("Prefer concise Chinese summaries", memory?.content)
            }
        } finally {
            sandbox.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    private fun requireIsolatedAvdOrTestUser() {
        val args = InstrumentationRegistry.getArguments()
        if (args.containsKey("isolatedUserId") || args.containsKey("isolatedAvdRunId")) {
            requireIsolatedProductTestUser(context)
            return
        }
        val isEmulator = Build.HARDWARE in setOf("ranchu", "goldfish") ||
            Build.FINGERPRINT.contains("generic", ignoreCase = true) ||
            Build.FINGERPRINT.contains("emulator", ignoreCase = true)
        check(isEmulator) {
            "Refusing destructive E2E setup on a non-emulator device without isolatedUserId/isolatedAvdRunId"
        }
    }

    private fun loadRealModelFixture(): RealModelFixture? {
        val args = InstrumentationRegistry.getArguments()
        val argEndpoint = args.getString("realLlmEndpoint")?.trim().orEmpty()
        val argModel = args.getString("realLlmModel")?.trim().orEmpty()
        val argKey = args.getString("realLlmApiKey")?.trim().orEmpty()
        val argProtocol = args.getString("realLlmProtocol")?.trim().orEmpty()
        if (argEndpoint.isNotEmpty() && argModel.isNotEmpty()) {
            return RealModelFixture(
                endpoint = argEndpoint,
                model = argModel,
                apiKey = argKey,
                protocol = argProtocol.ifEmpty { "chat_completions" },
            )
        }
        val candidateFiles = listOf(
            File("/data/local/tmp/nola-real-llm-e2e.json"),
            File(context.filesDir, "nola-real-llm-e2e.json"),
        )
        val file = candidateFiles.firstOrNull { it.isFile && it.canRead() } ?: return null
        val jsonObj = runCatching { JSONObject(file.readText()) }.getOrNull() ?: return null
        val endpoint = (jsonObj.optString("endpoint").ifBlank { jsonObj.optString("baseUrl") })
            .ifBlank { jsonObj.optString("base_url") }.trim()
        val model = jsonObj.optString("model").trim()
        val apiKey = jsonObj.optString("apiKey").ifBlank { jsonObj.optString("api_key") }.trim()
        val protocol = jsonObj.optString("protocol").ifBlank { jsonObj.optString("wireProtocol") }
            .trim().ifEmpty { "chat_completions" }
        if (endpoint.isEmpty() || model.isEmpty()) return null
        return RealModelFixture(endpoint = endpoint, model = model, apiKey = apiKey, protocol = protocol)
    }

    private suspend fun configureRealLanModel(
        runtime: AndroidApplicationRuntime,
        tokens: SecureTokenStore,
        memoryProfile: MemoryProfileDataStore,
        preferences: PreferenceDataStore,
        fixture: RealModelFixture,
    ) {
        val alreadyConfigured = runtime.listModelProfiles().any { profile ->
            profile.provider == "openai-compatible" &&
                profile.runtime == "remote" &&
                profile.deletedAtMs == null &&
                profile.enabled &&
                profile.modelName == fixture.model &&
                runCatching {
                    val src = Json.parseToJsonElement(profile.sourceJson).jsonObject
                    src["baseURL"]?.jsonPrimitive?.content == fixture.endpoint &&
                        src["wireProtocol"]?.jsonPrimitive?.content == fixture.protocol
                }.getOrDefault(false)
        }
        if (alreadyConfigured) {
            if (fixture.apiKey.isNotEmpty()) {
                tokens.saveLocalModelApiKey(fixture.apiKey)
            }
        } else {
            memoryProfile.saveAgentLlmConfig(
                endpoint = fixture.endpoint,
                apiKey = fixture.apiKey,
                model = fixture.model,
                wireProtocol = fixture.protocol,
            )
        }
        memoryProfile.configuredApiModelProfiles()
            .map(CanonicalModelProfile::toConfiguredModelInfo)
            .firstOrNull { it.id == fixture.model || it.name == fixture.model }
            ?.let { preferences.saveDefaultModelSelection(it) }
    }

    private fun waitForWorkspaceReady() {
        val skip = hasText(context.getString(R.string.onboarding_guide_action_later))
        composeRule.waitUntil(20_000) {
            composeRule.onAllNodes(primaryNavigation or skip).fetchSemanticsNodes().isNotEmpty()
        }
        if (composeRule.onAllNodes(primaryNavigation).fetchSemanticsNodes().isEmpty()) {
            composeRule.onNode(skip).performProductClick()
        }
        composeRule.waitUntil(20_000) {
            composeRule.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun selectTab(labelRes: Int) {
        val label = context.getString(labelRes)
        val destination = (hasText(label) or hasContentDescription(label)) and hasAnyAncestor(primaryNavigation)
        composeRule.waitUntil(15_000) {
            composeRule.onAllNodes(destination).fetchSemanticsNodes().isNotEmpty()
        }
        composeRule.onNode(destination).performProductClick()
        composeRule.waitUntil(10_000) {
            composeRule.onAllNodes(destination and isSelected()).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun confirmFixtureCapabilitiesIfPresent(modelId: String) {
        val rowMatcher = hasAnyAncestor(hasTestTag(MODEL_ROW_TEST_TAG_PREFIX + modelId))
        val capDesc = context.getString(R.string.models_execution_capabilities)
        val directCapMatcher = hasContentDescription(capDesc) and rowMatcher
        val moreMatcher = hasContentDescription(context.getString(R.string.intelligence_more)) and rowMatcher
        if (composeRule.onAllNodes(directCapMatcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()) {
            composeRule.onNode(directCapMatcher, useUnmergedTree = true).performProductScrollTo().performProductClick()
        } else if (composeRule.onAllNodes(moreMatcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()) {
            composeRule.onNode(moreMatcher, useUnmergedTree = true).performProductScrollTo().performProductClick()
            if (composeRule.onAllNodesWithText(capDesc).fetchSemanticsNodes().isEmpty()) return
            composeRule.onNodeWithText(capDesc).performProductClick()
        } else {
            return
        }
        val hint = context.getString(R.string.models_execution_capabilities_hint)
        composeRule.waitUntil(10_000) {
            composeRule.onAllNodesWithText(hint).fetchSemanticsNodes().isNotEmpty()
        }
        val save = composeRule.onNodeWithText(context.getString(R.string.common_save))
        if (save.fetchSemanticsNode().config.contains(SemanticsProperties.Disabled)) {
            composeRule.onNodeWithText(context.getString(R.string.common_cancel)).performProductScrollTo().performProductClick()
        } else {
            save.performProductScrollTo().performProductClick()
        }
        composeRule.waitUntil(10_000) {
            composeRule.onAllNodesWithText(hint).fetchSemanticsNodes().isEmpty()
        }
    }

    private fun captureScreenshot(fileName: String) {
        composeRule.waitForIdle()
        SystemClock.sleep(350)
        val dir = File(context.filesDir, "e2e-screenshots").apply { mkdirs() }
        val target = File(dir, fileName)
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot()) {
            "uiAutomation.takeScreenshot() returned null for $fileName"
        }
        try {
            target.outputStream().use { out ->
                check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)) {
                    "Failed to write screenshot PNG for $fileName"
                }
            }
        } finally {
            bitmap.recycle()
        }
    }

    private fun SemanticsNodeInteraction.performProductScrollTo(): SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try {
            performScrollTo()
            composeRule.mainClock.advanceTimeBy(300)
            composeRule.waitForIdle()
        } finally {
            composeRule.mainClock.autoAdvance = previous
        }
        return this
    }

    private fun awaitRealReply(expectedMarker: String) {
        // Drive Compose frames explicitly during streaming so active progress animations cannot deadlock waitForIdle.
        var renderedReply = ""
        var composerIdle = false
        val previousAutoAdvance = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = false
        try {
            val deadline = SystemClock.elapsedRealtime() + 90_000L
            while (SystemClock.elapsedRealtime() < deadline) {
                composeRule.mainClock.advanceTimeBy(64)
                val streamingNodes = composeRule
                    .onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                    .fetchSemanticsNodes()
                val sources = streamingNodes.mapNotNull {
                    it.config.getOrNull(ChatStreamingMarkdownSourceKey)
                        ?: it.config.getOrNull(StreamingMarkdownPresentedTextKey)
                }
                renderedReply = sources.firstOrNull { expectedMarker in it } ?: sources.lastOrNull().orEmpty()
                composerIdle = composeRule
                    .onAllNodesWithTag("chat.composer", useUnmergedTree = true)
                    .fetchSemanticsNodes()
                    .any { it.boundsInRoot.width > 0f && it.config.getOrNull(ChatComposerGenerationActiveKey) == false }
                if (renderedReply.contains(expectedMarker) && composerIdle) break
                SystemClock.sleep(100)
            }
            composeRule.mainClock.advanceTimeBy(256)
        } finally {
            composeRule.mainClock.autoAdvance = previousAutoAdvance
        }
        assertTrue(
            "Expected real LAN model reply containing $expectedMarker (idle=$composerIdle), got: $renderedReply",
            renderedReply.contains(expectedMarker) && composerIdle,
        )
    }

    private fun SemanticsNodeInteraction.performProductInput(text: String): SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try {
            performTextInput(text)
            composeRule.mainClock.advanceTimeBy(64)
            composeRule.waitForIdle()
        } finally {
            composeRule.mainClock.autoAdvance = previous
        }
        return this
    }

    private fun SemanticsNodeInteraction.performProductClick(): SemanticsNodeInteraction {
        val previous = composeRule.mainClock.autoAdvance
        composeRule.mainClock.autoAdvance = true
        try {
            performClick()
            composeRule.mainClock.advanceTimeBy(350)
            composeRule.waitForIdle()
        } finally {
            composeRule.mainClock.autoAdvance = previous
        }
        return this
    }
}
