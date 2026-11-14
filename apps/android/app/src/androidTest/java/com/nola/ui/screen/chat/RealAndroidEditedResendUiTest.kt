// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.os.SystemClock
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.core.AndroidCanonicalTurnDescriptor
import com.nola.core.loadAndroidCanonicalTerminalSeal
import com.nola.data.local.*
import com.nola.domain.projection.ConversationProjectionV1
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Creates completed real-provider sources through the actual isolated UI.
 * Only UI actions create and dispatch the edited intent. Reads verify exact
 * owner, frozen source and actual seals; no generated replies or receipts.
 */
@RunWith(AndroidJUnit4::class)
class RealAndroidEditedResendUiTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val clickedQaUiRequests = mutableSetOf<String>()

    @Test fun inlineCardPermissionIsVisibleApprovedAndRenderedByRealModel() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        AndroidApplicationRuntime(context).use { runtime -> createRealAnsweredSource(false, runtime, null, requireUiCard = true) }
    }

    @Test fun ordinaryEditRewindsSameOwnerAndReplacesSourceOutput() = exercise(false)
    @Test fun expertEditRewindsExistingChildrenAndKeepsSameParent() = exercise(true)

    private fun exercise(expert: Boolean) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val runtime = AndroidApplicationRuntime(context)
        runtime.use {
            val template = if (expert) createRealExpertTemplateFixture(context, runtime) else null
            val source = createRealAnsweredSource(expert, runtime, template)
            val originalManifest = if (expert) checkNotNull(runtime.expertMessageSource(source.first, source.second)) else null
            val changedCatalogRevision = if (template != null) {
                assertRealExpertTemplateCapture(runtime, checkNotNull(originalManifest), template)
                template.catalogRevision
            } else null
            var sourceBefore = source.third
            val conversationsBefore = runtime.listConversations().map { it.id }.toSet()
            val marker = "EDIT-REAL-" + java.util.UUID.randomUUID().toString().take(8)
            val editedText = "Reply briefly with $marker and one sentence about a fictional garden."
            val report = JSONObject().put("expert", expert).put("sourceConversation", source.first).put("sourceMessage", source.second)
                .put("templateFixtureID", template?.id).put("sourceDefinitionSha256", template?.digest)
                .put("catalogUpdatedRevision", changedCatalogRevision).put("actualBeginDefinitionConfirmed", template != null)
            // Source creation deliberately drives a paused clock. Restore frame
            // progression for the next Activity's real splash/navigation entry.
            compose.mainClock.autoAdvance = true
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                scenario.onActivity { activity ->
                    activity.setTurnScreenOn(true)
                    activity.setShowWhenLocked(true)
                    activity.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
                compose.waitUntil(20_000) { compose.onAllNodesWithTag("navigation.bottom").fetchSemanticsNodes().isNotEmpty() }
                compose.mainClock.autoAdvance = false
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).clickProduct()
                if (compose.onAllNodesWithTag("chat.conversation.back").fetchSemanticsNodes().isNotEmpty())
                    compose.onNodeWithTag("chat.conversation.back").clickProduct()
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("chat.conversations.list").fetchSemanticsNodes().isNotEmpty() }
                compose.mainClock.autoAdvance = true
                compose.onNodeWithTag("chat.conversations.search").performTextReplacement("")
                compose.waitUntil(15_000) {
                    runCatching {
                        compose.onNodeWithTag("chat.conversations.list").performScrollToNode(hasTestTag("chat.conversation.${source.first}"))
                        true
                    }.getOrDefault(false)
                }
                compose.mainClock.autoAdvance = false
                compose.onNodeWithTag("chat.conversation.${source.first}").clickProduct()
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("chat-transcript-list").fetchSemanticsNodes().isNotEmpty() }
                val action = "chat.message.edit.${source.second}"
                compose.mainClock.autoAdvance = true
                compose.onNodeWithTag("chat-transcript-list").performScrollToNode(hasTestTag(action))
                compose.mainClock.autoAdvance = false
                // Capture the immutable-source baseline immediately before the
                // first editor opens, after this exact answered source is shown.
                sourceBefore = runtime.loadConversationProjectionV1(source.first)
                if (originalManifest != null) {
                    check(originalManifest.entries.all { entry -> sourceHasAnsweredSeal(runtime,
                        entry.participant.contextConversationId, entry.childUserMessageId, entry.childAssistantMessageId) })
                } else {
                    val userIndex = sourceBefore.messages.indexOfFirst { it.messageId == source.second }
                    check(userIndex >= 0)
                    val assistant = sourceBefore.messages.getOrNull(userIndex + 1)
                    check(assistant?.role == ConversationProjectionV1.Role.Assistant &&
                        sourceHasAnsweredSeal(runtime, source.first, source.second, assistant.messageId))
                }
                compose.onNodeWithTag(action).assertIsDisplayed().clickProduct()
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("chat.message.editor.input").fetchSemanticsNodes().isNotEmpty() }
                // Real cancel must leave both source and target set untouched.
                compose.onNodeWithTag("chat.message.editor.input").performTextReplacement("unsaved edit")
                compose.onNodeWithTag("chat.message.editor.cancel").clickProduct()
                check(runtime.listConversations().map { it.id }.toSet() == conversationsBefore)
                assertSourceUnchanged(runtime, sourceBefore)
                compose.onNodeWithTag(action).clickProduct()
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("chat.message.editor.input").fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithTag("chat.message.editor.input").performTextReplacement(editedText)
                compose.onNodeWithTag("chat.message.editor.submit").assertIsEnabled().clickProduct()
                var target: String? = null
                var expectedAnswers: List<String> = emptyList()
                val deadline = SystemClock.elapsedRealtime() + 240_000
                while (SystemClock.elapsedRealtime() < deadline) {
                    compose.mainClock.advanceTimeBy(64)
                    val explicitFailure = compose.onAllNodesWithTag("chat.message.editor.error", useUnmergedTree = true).fetchSemanticsNodes()
                    if (explicitFailure.isNotEmpty()) {
                        File(context.filesDir, "real-android-${if (expert) "expert" else "ordinary"}-edit-failure.txt")
                            .writeText(compose.onAllNodes(isRoot(), useUnmergedTree = true).fetchSemanticsNodes().indices
                                .joinToString("\n") { index -> compose.onAllNodes(isRoot(), useUnmergedTree = true)[index].printToString(maxDepth = 100) })
                        val message = explicitFailure.flatMap { it.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text).orEmpty() }
                            .joinToString(" ") { it.text }
                        error("The production editor explicitly rejected this resend: $message; see exact UI failure evidence")
                    }
                    val projection = runtime.loadConversationProjectionV1(source.first)
                    val editedUser = projection.messages.singleOrNull { message -> message.role == ConversationProjectionV1.Role.User &&
                        message.parts.filterIsInstance<ConversationProjectionV1.Part.Text>().singleOrNull()?.markdown == editedText }
                    if (editedUser != null) {
                        val manifest = if (expert) runtime.expertMessageSource(source.first, editedUser.messageId) else null
                        val owners = if (manifest != null) {
                            check(manifest.input.roomId == checkNotNull(originalManifest).input.roomId)
                            check(manifest.entries.map { it.participant.contextConversationId } == originalManifest.entries.map { it.participant.contextConversationId })
                            manifest.entries.map { Triple(it.participant.contextConversationId,it.childSubmissionId,it.childUserMessageId) }
                        } else if (!expert) listOf(Triple(source.first,editedUser.messageId.removeSuffix(":user"),editedUser.messageId)) else emptyList()
                        owners.forEach { (owner,run,_) -> approvePendingQaUi(runtime,owner,"$run:assistant",run) }
                        val texts = owners.mapNotNull { (owner,run,user) -> answeredText(runtime,owner,run,user,editedText) }
                        if (owners.isNotEmpty() && texts.size == owners.size && texts.all { marker in it }) {
                            target=source.first; expectedAnswers=texts
                        }
                    }
                    if (target != null) break
                    Thread.sleep(60)
                }
                checkNotNull(target) { "No exact real answered edited target within deadline" }
                check(target == source.first)
                check(runtime.loadConversationProjectionV1(source.first).messages.none { it.messageId == source.second })
                if (expert) check(runtime.expertMessageSource(source.first, source.second) == originalManifest)
                val targetProjection = runtime.loadConversationProjectionV1(target!!)
                check(targetProjection.messages.count { it.role == ConversationProjectionV1.Role.User } == 1)
                check(targetProjection.messages.filter { it.role == ConversationProjectionV1.Role.User }.single().parts
                    .filterIsInstance<ConversationProjectionV1.Part.Text>().single().markdown == editedText)
                // Short actual answers must be presented, not merely durable.
                compose.waitUntil(20_000) {
                    compose.mainClock.advanceTimeBy(64)
                    val visible = compose.onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                        .fetchSemanticsNodes().mapNotNull { it.config.getOrNull(ChatStreamingMarkdownSourceKey) }
                    expectedAnswers.all { answer -> visible.any { it == answer } }
                }
                report.put("targetConversation", target).put("answeredChildren", expectedAnswers.size).put("passed", true)
                scenario.recreate()
                compose.waitUntil(20_000) {
                    compose.mainClock.advanceTimeBy(64)
                    val visible = compose.onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true)
                        .fetchSemanticsNodes().mapNotNull { it.config.getOrNull(ChatStreamingMarkdownSourceKey) }
                    expectedAnswers.all { answer -> visible.any { it == answer } }
                }
                check(target == source.first)
                check(runtime.loadConversationProjectionV1(source.first).messages.none { it.messageId == source.second })
            }
            File(context.filesDir, "real-android-${if (expert) "expert" else "ordinary"}-edit-report.json").writeText(report.toString(2))
        }
    }

    private fun createRealAnsweredSource(expert: Boolean, runtime: AndroidApplicationRuntime, template: RealExpertTemplateFixture?, requireUiCard: Boolean = false): Triple<String, String, ConversationProjectionV1> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val fixture = JSONObject(File(context.filesDir, "nola-real-llm-e2e.json").readText())
        val modelName = fixture.getString("model")
        val tokens = com.nola.data.local.security.SecureTokenStore(context)
        runBlocking {
            com.nola.data.local.datastore.PreferenceDataStore(context, tokens).setOnboardingCompleted()
            applyRealModelFixture(context, runtime, fixture)
        }
        val before = runtime.listConversations().map { it.id }.toSet()
        val marker = "SOURCE-" + java.util.UUID.randomUUID().toString().take(8)
        val prompt = if (requireUiCard) "Show a small inline result card with the title $marker and brief text saying 37 times 19 is 703. Use only a simple text-and-column layout, no actions or external resources. Then reply briefly with $marker and the result."
            else "Calculate 37 times 19. Reply briefly with $marker and the result."
        var source: Triple<String, String, ConversationProjectionV1>? = null
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setTurnScreenOn(true); activity.setShowWhenLocked(true)
                activity.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
            compose.mainClock.autoAdvance = false
            compose.waitUntil(20_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("navigation.bottom").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).clickProduct()
            compose.onNodeWithText(context.getString(R.string.models_title)).clickProduct()
            val rowTag = com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX + modelName
            compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag(rowTag).fetchSemanticsNodes().isNotEmpty() }
            val use = compose.onAllNodes(hasText(context.getString(R.string.models_use)) and hasAnyAncestor(hasTestTag(rowTag)))
            if (use.fetchSemanticsNodes().isNotEmpty()) {
                compose.mainClock.autoAdvance = true
                use[0].performScrollTo().assertIsDisplayed().performClick()
                compose.mainClock.autoAdvance = false
                compose.mainClock.advanceTimeBy(400)
            }
            compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodes(hasText(context.getString(R.string.models_selected)) and hasAnyAncestor(hasTestTag(rowTag))).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).clickProduct()
            if (compose.onAllNodesWithTag("chat.conversation.back").fetchSemanticsNodes().isNotEmpty())
                compose.onNodeWithTag("chat.conversation.back").clickProduct()
            compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("chat.conversations.new").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag("chat.conversations.new").clickProduct()
            compose.waitUntil(15_000) {
                compose.mainClock.advanceTimeBy(32)
                compose.onAllNodesWithTag("chat.modelPicker").fetchSemanticsNodes().any { node ->
                    node.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text)
                        ?.any { modelName in it.text } == true
                }
            }
            if (expert) {
                val expertName = "Real edit expert " + System.nanoTime().toString().takeLast(7)
                compose.onNodeWithTag("chat.topbar.more").clickProduct()
                compose.onNodeWithTag("chat.experts.open").clickProduct()
                compose.waitUntil(15_000) {
                    compose.mainClock.advanceTimeBy(32)
                    compose.onAllNodesWithTag("chat.experts.manage").fetchSemanticsNodes().isNotEmpty()
                }
                compose.onNodeWithTag("chat.experts.manage").clickProduct()
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("expert.create").fetchSemanticsNodes().any { !it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) } }
                compose.onNodeWithTag("expert.create").assertIsEnabled().clickProduct()
                compose.onNodeWithTag("expert.editor.name").performTextInput(expertName)
                compose.onNodeWithTag("expert.editor.description").performTextInput("Answers the requested short arithmetic task.")
                if (template != null) {
                    compose.mainClock.autoAdvance = true
                    compose.onNodeWithTag("expert.editor.template").performScrollTo().assertIsDisplayed().performClick()
                    compose.waitUntil(15_000) { compose.onAllNodesWithTag("expert.template.${template.id}").fetchSemanticsNodes().isNotEmpty() }
                    compose.onNodeWithTag("expert.template.${template.id}").assertIsDisplayed().performClick()
                    compose.mainClock.autoAdvance = false
                }
                compose.mainClock.autoAdvance = true
                compose.onNodeWithTag("expert.editor.save").performScrollTo().assertIsDisplayed().performClick()
                compose.mainClock.autoAdvance = false
                compose.mainClock.advanceTimeBy(400)
                compose.waitUntil(15_000) { compose.mainClock.advanceTimeBy(32); compose.onAllNodesWithTag("expert.editor.save").fetchSemanticsNodes().isEmpty() }
                compose.onNodeWithText(context.getString(R.string.common_back)).clickProduct()
                compose.mainClock.autoAdvance = true
                compose.onNodeWithTag("chat.experts.list").performScrollToNode(hasText(expertName))
                compose.mainClock.autoAdvance = false
                compose.onNodeWithText(expertName).clickProduct()
                compose.onNodeWithText(context.getString(R.string.common_done)).clickProduct()
            }
            compose.waitUntil(15_000) {
                compose.mainClock.advanceTimeBy(32)
                compose.onAllNodesWithTag("chat.composer.input").fetchSemanticsNodes()
                    .any { !it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled) }
            }
            compose.onNodeWithTag("chat.composer.input").performTextInput(prompt)
            compose.mainClock.advanceTimeBy(64)
            compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).assertIsEnabled().clickProduct()
            val deadline = SystemClock.elapsedRealtime() + 240_000
            while (SystemClock.elapsedRealtime() < deadline && source == null) {
                compose.mainClock.advanceTimeBy(64)
                for (conversation in runtime.listConversations().filter { it.id !in before }) {
                    val projection = runtime.loadConversationProjectionV1(conversation.id)
                    val index = projection.messages.indexOfFirst { message ->
                        message.role == ConversationProjectionV1.Role.User && message.parts.filterIsInstance<ConversationProjectionV1.Part.Text>().singleOrNull()?.markdown == prompt
                    }
                    if (index < 0) continue
                    val user = projection.messages[index]
                    val manifest = runtime.expertMessageSource(conversation.id, user.messageId)
                    if (expert) {
                        if (manifest == null || manifest.entries.size != 1) continue
                        val entry = manifest.entries.single()
                        val receipt = com.nola.data.repository.AgentRoomRepository(runtime).submission(manifest.input.roomId, entry.childSubmissionId)
                        if (receipt == null || receipt.executionId != entry.childSubmissionId ||
                            receipt.contextConversationId != entry.participant.contextConversationId) continue
                        approvePendingQaUi(runtime, entry.participant.contextConversationId, entry.childAssistantMessageId, entry.childSubmissionId)
                        val childAnswered = sourceHasAnsweredSeal(runtime, entry.participant.contextConversationId,
                            entry.childUserMessageId, entry.childAssistantMessageId)
                        if (receipt.state !in setOf("completed", "completed_with_warnings") ||
                            entry.modelWitness == null || entry.instructionWitness == null || !childAnswered) continue
                        val answer = runtime.loadConversationProjectionV1(entry.participant.contextConversationId).messages
                            .single { it.messageId == entry.childAssistantMessageId }.parts.filterIsInstance<ConversationProjectionV1.Part.Text>().joinToString("\n") { it.markdown }
                        if (marker !in answer) continue
                    } else {
                        if (manifest != null) continue
                        val assistant = projection.messages.getOrNull(index + 1) ?: continue
                        if (assistant.role != ConversationProjectionV1.Role.Assistant) continue
                        approvePendingQaUi(runtime, conversation.id, assistant.messageId)
                        if (!sourceHasAnsweredSeal(runtime, conversation.id, user.messageId, assistant.messageId)) continue
                        if (marker !in assistant.parts.filterIsInstance<ConversationProjectionV1.Part.Text>().joinToString("\n") { it.markdown }) continue
                    }
                    source = Triple(conversation.id, user.messageId, projection)
                }
                if (source == null) Thread.sleep(60)
            }
            checkNotNull(source) { "The new real source did not reach an exact answered seal" }
            compose.waitUntil(20_000) {
                compose.mainClock.advanceTimeBy(64)
                compose.onAllNodesWithTag(CHAT_STREAMING_MARKDOWN_TEST_TAG, useUnmergedTree = true).fetchSemanticsNodes()
                    .any { marker in it.config.getOrNull(ChatStreamingMarkdownSourceKey).orEmpty() }
            }
            if (requireUiCard) {
                val current = runtime.loadConversationProjectionV1(checkNotNull(source).first)
                val userIndex = current.messages.indexOfFirst { it.messageId == source!!.second }
                val assistant = current.messages[userIndex + 1]
                val tool = assistant.parts.filterIsInstance<ConversationProjectionV1.Part.Tool>().single { it.capabilityId == "render_ui" }
                val permission = checkNotNull(tool.permission)
                check(permission.requestId in clickedQaUiRequests && permission.decision == "allow_once" &&
                    tool.state == ConversationProjectionV1.ActivityState.Completed) { "Actual one-off Core permission and render completion were not confirmed" }
                val surface = assistant.parts.filterIsInstance<ConversationProjectionV1.Part.UiSurface>().single().surface
                check(surface.identity.owner.conversation_id == current.conversationId &&
                    surface.identity.owner.message_id == assistant.messageId)
                compose.mainClock.autoAdvance = true
                val ownerMatcher = SemanticsMatcher.expectValue(com.nola.ui.generative.UiSurfaceIdentitySemantics, surface.identity)
                compose.onNodeWithTag("chat-transcript-list").performTouchInput { swipeDown() }
                compose.onNodeWithTag("chat-transcript-list").performScrollToNode(ownerMatcher)
                compose.onNode(ownerMatcher, useUnmergedTree = true).assertIsDisplayed()
                compose.onNode(hasText(marker, substring = true) and hasAnyAncestor(ownerMatcher), useUnmergedTree = true).assertIsDisplayed()
                compose.onNode(hasText("703", substring = true) and hasAnyAncestor(ownerMatcher), useUnmergedTree = true).assertIsDisplayed()
                val renderedRequest = tool.input.jsonObject
                validateQaUiArguments(renderedRequest)
                File(context.filesDir, "real-inline-surface-request.json").writeText(renderedRequest.toString())
                val screenshot = File(context.filesDir, "real-inline-permission-rendered.png")
                screenshot.outputStream().use { output -> InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                    .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output) }
                File(context.filesDir, "real-inline-permission-report.json").writeText(JSONObject()
                    .put("conversation_id", current.conversationId).put("assistant_message_id", assistant.messageId)
                    .put("request_id", permission.requestId).put("decision", permission.decision)
                    .put("tool_call_id", tool.toolCallId).put("surface_id", surface.identity.surface_id)
                    .put("generation", surface.identity.generation).put("surface_revision", surface.revision)
                    .put("arguments_sha256", RealExpertTemplateFixture.sha(renderedRequest.toString()))
                    .put("actual_answered_seal", true).put("rendered_card_visible", true).toString(2))
            }
        }
        return checkNotNull(source)
    }

    /** This synthetic QA task permits one-off local, inert text/column rendering.
     * Concrete Core ownership and schema authorize the test click, never labels
     * or words in the user's prompt. Any other capability fails explicitly. */
    private fun approvePendingQaUi(runtime: AndroidApplicationRuntime, conversation: String, assistant: String, expectedRun: String? = null) {
        val active = runtime.executeQuery(buildJsonObject { put("type", "active_run"); put("conversation_id", conversation) })
        if (active == JsonNull) return
        val owner = active.jsonObject
        val run = owner.getValue("run_id").jsonPrimitive.content
        check((expectedRun == null || run == expectedRun) && owner["assistant_message_id"] == JsonPrimitive(assistant))
        val value = runtime.executeQuery(buildJsonObject {
            put("type", "load_conversation_permission_projection_v1"); put("conversation_id", conversation); put("run_id", run)
            put("scope", buildJsonObject { put("type", "current_run") })
        })
        if (value == JsonNull) return
        val projection = value.jsonObject
        check(projection["conversation_id"] == JsonPrimitive(conversation) && projection["run_id"] == JsonPrimitive(run) &&
            projection["active_turn_id"] == JsonPrimitive(run))
        for (raw in projection.getValue("permissions").jsonObject.getValue("pending").jsonArray) {
            val pending = raw.jsonObject
            val id = pending.getValue("request_id").jsonPrimitive.content
            if (id in clickedQaUiRequests) continue
            check(pending["turn_id"] == JsonPrimitive(run) && pending["capability_id"] == JsonPrimitive("render_ui") &&
                pending["permission_domain"] == JsonPrimitive("render_ui") &&
                pending["continuation"] == buildJsonObject { put("type", "core_tool") } &&
                (pending["invocation_context"] == null || pending["invocation_context"] == JsonNull)) { "Unexpected concrete QA permission capability" }
            val args = pending.getValue("arguments").jsonObject
            check(Json.parseToJsonElement(pending.getValue("resource").jsonPrimitive.content) == args)
            validateQaUiArguments(args)
            val canonical = runtime.loadConversationProjectionV1(conversation).messages.single { it.messageId == assistant }
            val tool = canonical.parts.filterIsInstance<ConversationProjectionV1.Part.Tool>().single {
                it.toolCallId == pending.getValue("call_id").jsonPrimitive.content
            }
            val permission = checkNotNull(tool.permission)
            check(tool.capabilityId == "render_ui" && tool.origin.kind == "builtin" && tool.origin.providerId == "nola" &&
                tool.origin.namespace == "generative_ui" && permission.requestId == id &&
                permission.permissionDomain == "render_ui" && permission.decision == null &&
                tool.state == ConversationProjectionV1.ActivityState.WaitingForPermission && tool.input == args)
            val tag = "permission.allow-once.$id"
            if (compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isEmpty()) return
            val auto = compose.mainClock.autoAdvance
            compose.mainClock.autoAdvance = true
            try {
                val button = compose.onNodeWithTag(tag, useUnmergedTree = true)
                button.performScrollTo().assertIsDisplayed()
                val bounds = button.fetchSemanticsNode().boundsInWindow
                val viewport = compose.onNodeWithTag("chat-transcript-list").fetchSemanticsNode().boundsInWindow
                val composer = compose.onNodeWithTag("chat.composer").fetchSemanticsNode().boundsInWindow
                check(bounds.top >= viewport.top - 1 && bounds.bottom <= viewport.bottom + 1 && bounds.bottom <= composer.top + 1) {
                    "The exact Allow once action is obscured by the composer or outside the actual transcript viewport"
                }
                val suffix = RealExpertTemplateFixture.sha(id).take(12)
                val context = InstrumentationRegistry.getInstrumentation().targetContext
                File(context.filesDir, "real-edit-permission-$suffix.png").outputStream().use { output ->
                    InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
                        .compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
                }
                File(context.filesDir, "real-edit-permission-$suffix.json").writeText(JSONObject()
                    .put("conversation_id", conversation).put("run_id", run).put("assistant_message_id", assistant)
                    .put("request_id", id).put("call_id", tool.toolCallId).put("capability_id", "render_ui")
                    .put("arguments_sha256", RealExpertTemplateFixture.sha(args.toString()))
                    .put("button_bottom", bounds.bottom.toDouble()).put("viewport_bottom", viewport.bottom.toDouble())
                    .put("composer_top", composer.top.toDouble()).toString(2))
                // Revalidate the exact Core control projection after scrolling;
                // clicking is evidence of the UI action, not proof of a grant.
                val current = runtime.executeQuery(buildJsonObject {
                    put("type", "load_conversation_permission_projection_v1"); put("conversation_id", conversation); put("run_id", run)
                    put("scope", buildJsonObject {
                        put("type", "control")
                        for (key in listOf("session_id", "active_turn_id", "event_cursor", "permission_projection_revision"))
                            put(key, projection.getValue(key))
                    })
                })
                check(current != JsonNull && current.jsonObject.getValue("permissions").jsonObject.getValue("pending").jsonArray.any { it == raw })
                button.performClick()
                clickedQaUiRequests += id
            } finally { compose.mainClock.autoAdvance = auto }
        }
    }

    private fun validateQaUiArguments(args: JsonObject) {
        check(args.keys == setOf("expected_revision", "messages") && args["expected_revision"] == JsonPrimitive(0))
        val messages = args.getValue("messages").jsonArray
        check(messages.size in 1..8)
        var created: String? = null
        for (message in messages) {
            val fields = message.jsonObject
            check(fields["version"] == JsonPrimitive("v0.9.1") && fields.size == 2)
            when {
                "createSurface" in fields -> {
                    check(created == null)
                    val body = fields.getValue("createSurface").jsonObject
                    check(body.keys == setOf("surfaceId", "catalogId") &&
                        body["catalogId"] == JsonPrimitive("https://nola.app/a2ui/v0.9.1/catalog.json"))
                    created = body.getValue("surfaceId").jsonPrimitive.content.also { check(it.isNotBlank()) }
                }
                "updateComponents" in fields -> {
                    val body = fields.getValue("updateComponents").jsonObject
                    check(created != null && body.keys == setOf("surfaceId", "components") && body["surfaceId"] == JsonPrimitive(created))
                    val components = body.getValue("components").jsonArray
                    check(components.size in 1..16)
                    for (value in components) {
                        val component = value.jsonObject
                        check(component.getValue("id").jsonPrimitive.content.isNotBlank())
                        when (component.getValue("component").jsonPrimitive.content) {
                            "Column" -> check(component.keys == setOf("id", "component", "children") &&
                                component.getValue("children").jsonArray.all { it.jsonPrimitive.isString })
                            "Text" -> check(component.keys.containsAll(setOf("id", "component", "text")) &&
                                (component.keys - setOf("id", "component", "text", "variant")).isEmpty() &&
                                (component["variant"] == null || component["variant"] in setOf("h1", "h2", "h3", "h4", "h5", "caption", "body").map(::JsonPrimitive)) &&
                                component.getValue("text").jsonPrimitive.isString && component.getValue("text").jsonPrimitive.content.length <= 4096)
                            else -> error("Unexpected concrete QA UI component")
                        }
                    }
                }
                "updateDataModel" in fields -> {
                    val body = fields.getValue("updateDataModel").jsonObject
                    check(created != null && body.keys == setOf("surfaceId", "value") && body["surfaceId"] == JsonPrimitive(created) &&
                        body.getValue("value") == buildJsonObject {})
                }
                else -> error("Unexpected concrete QA UI operation")
            }
        }
        check(created != null)
    }

    private fun assertSourceUnchanged(runtime: AndroidApplicationRuntime, before: ConversationProjectionV1) {
        val after = runtime.loadConversationProjectionV1(before.conversationId)
        if (after == before) return
        val evidence = JSONObject().put("conversation_id", before.conversationId)
            .put("before_revision", before.revision).put("after_revision", after.revision)
            .put("title_equal", before.title == after.title).put("messages_equal", before.messages == after.messages)
            .put("active_run_equal", before.activeRun == after.activeRun)
            .put("context_usage_equal", before.contextUsage == after.contextUsage)
            .put("surface_equal", before.surface == after.surface)
            .put("message_metadata", org.json.JSONArray(before.messages.map { prior ->
                val current = after.messages.singleOrNull { it.messageId == prior.messageId }
                JSONObject().put("message_id", prior.messageId).put("before_state", prior.state.name)
                    .put("after_state", current?.state?.name).put("before_revision", prior.revision)
                    .put("after_revision", current?.revision).put("parts_equal", prior.parts == current?.parts)
            }))
        File(InstrumentationRegistry.getInstrumentation().targetContext.filesDir,
            "real-android-edit-source-changed.json").writeText(evidence.toString(2))
        error("Source projection changed; exact safe field-difference evidence retained")
    }

    private fun sourceHasAnsweredSeal(runtime: AndroidApplicationRuntime, conversation: String, user: String, assistant: String): Boolean {
        val projection = runtime.loadConversationProjectionV1(conversation)
        val message = projection.messages.singleOrNull { it.messageId == assistant } ?: return false
        val window = runtime.loadConversationProjectionWindowV1(conversation, null, 80, 512_000)
        val head = window.assistantJournalHeads.singleOrNull {
            it["identity"]?.jsonObject?.get("assistant_message_id")?.jsonPrimitive?.content == message.messageId
        }?.getValue("identity")?.jsonObject ?: return false
        if (head.getValue("conversation_id").jsonPrimitive.content != conversation) return false
        val run = head.getValue("run_id").jsonPrimitive.content
        val exact = runtime.executeQuery(buildJsonObject {
            put("type", "load_exact_run_invocation"); put("conversation_id", conversation)
            put("expected_run_id", run); put("assistant_message_id", assistant)
        }).jsonObject
        if (exact["status"]?.jsonPrimitive?.content != "sealed") return false
        val userText = projection.messages.single { it.messageId == user }.parts
            .filterIsInstance<ConversationProjectionV1.Part.Text>().joinToString("\n") { it.markdown }
        val descriptor = AndroidCanonicalTurnDescriptor(conversation, "", run, user, userText, null, assistantMessageId = assistant)
        val seal = loadAndroidCanonicalTerminalSeal(descriptor, exact.getValue("core_session_id").jsonPrimitive.content, runtime::executeQuery) ?: return false
        val resultKind = seal.result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content
        if (resultKind != "answered") {
            val evidence = JSONObject().put("conversation_id", conversation).put("user_message_id", user)
                .put("assistant_message_id", assistant).put("run_id", run)
                .put("message_state", message.state.name).put("sealed_result_kind", resultKind)
                .put("failure_codes", org.json.JSONArray(message.parts.filterIsInstance<ConversationProjectionV1.Part.ModelAttempt>()
                    .map { it.failure.code }))
            File(InstrumentationRegistry.getInstrumentation().targetContext.filesDir,
                "real-android-edit-source-terminal-failure.json").writeText(evidence.toString(2))
            error("The exact real source sealed without an answered result: $resultKind; Run=$run")
        }
        return message.state == ConversationProjectionV1.MessageState.Completed
    }

    private fun answeredText(runtime: AndroidApplicationRuntime, owner: String, run: String, user: String, text: String): String? {
        val projection = runtime.loadConversationProjectionV1(owner)
        val message = projection.messages.singleOrNull { it.messageId == "$run:assistant" && it.state == ConversationProjectionV1.MessageState.Completed } ?: return null
        val exact = runtime.executeQuery(buildJsonObject {
            put("type", "load_exact_run_invocation"); put("conversation_id", owner)
            put("expected_run_id", run); put("assistant_message_id", "$run:assistant")
        }).jsonObject
        if (exact["status"]?.jsonPrimitive?.content != "sealed") return null
        val descriptor = AndroidCanonicalTurnDescriptor(owner, "Edited",
            run, user, text, null, assistantMessageId = "$run:assistant")
        val seal = loadAndroidCanonicalTerminalSeal(descriptor, exact.getValue("core_session_id").jsonPrimitive.content, runtime::executeQuery) ?: return null
        check(seal.result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content == "answered")
        return message.parts.filterIsInstance<ConversationProjectionV1.Part.Text>().joinToString("\n") { it.markdown }
    }

    private fun SemanticsNodeInteraction.clickProduct() {
        performClick()
        compose.mainClock.advanceTimeBy(400)
        compose.waitForIdle()
    }
}
