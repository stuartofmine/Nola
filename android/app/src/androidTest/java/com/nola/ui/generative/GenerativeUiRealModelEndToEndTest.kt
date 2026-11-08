package com.nola.ui.generative

import android.os.SystemClock
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.ui.screen.chat.ChatTranscriptMessageIds
import com.nola.ui.screen.chat.ChatTimelineMotionPolicy
import com.nola.llm.provider.WebAgentProvider
import com.nola.data.remote.api.*
import com.nola.MainActivity
import com.nola.R
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.UiSnapshot
import com.nola.data.local.UiSurfacePhase
import com.nola.data.local.listUiSurfaces
import com.nola.data.local.remoteUiEpoch
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Opt-in disposable emulator acceptance. All UI comes from the real provider. */
@RunWith(AndroidJUnit4::class)
class GenerativeUiRealModelEndToEndTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val fixture get() = File(context.filesDir, "a2ui-real-model-fixture.json")
    private var remoteConnection: WebAgentProvider.Config? = null
    private var remoteProvider: WebAgentProvider? = null
    private var remoteHttp: io.ktor.client.HttpClient? = null
    private val remoteAdmissions = mutableListOf<JsonObject>()
    private val remoteRunKeys = mutableMapOf<String, RemoteRunKey>()
    private val admittedSourceConversations = mutableSetOf<String>()
    private fun surfaces(app: AndroidApplicationRuntime, conversation: String): List<UiSnapshot> =
        remoteConnection?.let { connection -> runBlocking { app.remoteUiEpoch(conversation,connection.expectedHostId)?.let { epoch -> requireNotNull(remoteProvider).listUiSurfaces(conversation,connection,epoch) } ?: emptyList() } }
            ?: app.listUiSurfaces(conversation)

    @Test fun clearFixtureCredential() {
        check(InstrumentationRegistry.getArguments().getString("disposableA2uiFixture") == "true")
        SecureTokenStore(context).deleteLocalModelApiKey()
        runBlocking { PreferenceDataStore(context,SecureTokenStore(context)).clearWebHostConfig() }
        fixture.delete()
        check(context.getSharedPreferences("nola_secure_tokens", android.content.Context.MODE_PRIVATE).edit().commit())
    }

    @Test fun realProviderFormEditsSubmitsUpdatesAndReopens() {
        check(InstrumentationRegistry.getArguments().getString("disposableA2uiFixture") == "true")
        check(fixture.isFile) { "Stage the isolated real model fixture first" }
        val config = JSONObject(fixture.readText())
        check(config.getBoolean("disposableFixture"))
        val tokenStore = SecureTokenStore(context)
        try {
            AndroidApplicationRuntime(context).use { application ->
                runBlocking {
                    val preferences = PreferenceDataStore(context, tokenStore)
                    preferences.setOnboardingCompleted()
                    if (config.optBoolean("remoteHost")) {
                        preferences.saveWebHostConfig(config.getString("url"),config.getString("token"))
                        check(preferences.trustWebHostIdentity(config.getString("url"),config.getString("hostId")))
                        remoteConnection=WebAgentProvider.Config(hostBaseUrl=config.getString("url"),token=config.getString("token"),
                            expectedHostId=config.getString("hostId"),clientId=preferences.getOrCreateClientUuid())
                        val http=com.nola.di.NetworkModule.provideKtorClient()
                        remoteHttp=http
                        remoteProvider=WebAgentProvider(WebHostApiServiceImpl(http,com.nola.data.remote.sse.SseClient(http),WebHostHttpClientFactory(http)),
                            com.nola.llm.provider.ResourceWebAgentSummaryTextSource(context))
                    } else MemoryProfileDataStore(context, tokenStore, application).saveAgentLlmConfig(
                        config.getString("endpoint"), config.getString("apiKey"), config.getString("model"), config.getString("wireProtocol"))
                }
            }
            fixture.delete() // The credential is now only in the isolated Android Keystore.
            val before = AndroidApplicationRuntime(context).use { it.listConversations().map { row -> row.id }.toSet() }
            val resumeConversation = config.optString("resumeConversation", "")
            var conversation = ""
            var updated: UiSnapshot? = null
            var initialSeal: JsonObject? = null
            var submittedSeal: JsonObject? = null
            var submittedRunId: String? = null
            var initialRevision = 0L
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                compose.waitUntil(30_000) { compose.onAllNodesWithContentDescription(context.getString(R.string.tab_settings)).fetchSemanticsNodes().isNotEmpty() }
                if (remoteConnection == null) {
                compose.onNodeWithContentDescription(context.getString(R.string.tab_settings)).performClick()
                compose.onNodeWithText(context.getString(R.string.models_title)).performClick()
                compose.waitUntil(20_000) { compose.onAllNodesWithTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model")).fetchSemanticsNodes().isNotEmpty() }
                val modelRow = hasAnyAncestor(hasTestTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model")))
                compose.onNode(hasContentDescription(context.getString(R.string.models_execution_capabilities)) and modelRow).performScrollTo().performClick()
                val tools = compose.onNode(hasText(context.getString(R.string.models_execution_tools)) and isToggleable())
                if (tools.fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.ToggleableState] != androidx.compose.ui.state.ToggleableState.On) tools.performClick()
                val saveCapabilities = compose.onNodeWithText(context.getString(R.string.common_save))
                if (!saveCapabilities.fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled)) saveCapabilities.performScrollTo().performClick()
                else compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
                compose.waitUntil(5_000) { compose.onAllNodesWithText(context.getString(R.string.models_execution_tools)).fetchSemanticsNodes().isEmpty() }
                val useModel = compose.onAllNodes(hasText(context.getString(R.string.models_use)) and
                    hasAnyAncestor(hasTestTag(MODEL_ROW_TEST_TAG_PREFIX + config.getString("model"))))
                if (useModel.fetchSemanticsNodes().isNotEmpty()) useModel.onFirst().performClick()
                }
                compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                if (resumeConversation.isNotBlank()) {
                    check(remoteConnection != null) { "Resume fixture must be a source-Host conversation" }
                    compose.waitUntil(20_000) { compose.onAllNodesWithTag("chat.conversation.$resumeConversation").fetchSemanticsNodes().isNotEmpty() }
                    compose.onNodeWithTag("chat.conversation.$resumeConversation").performScrollTo().performClick()
                } else {
                compose.onNodeWithTag("chat.conversations.new").performClick()
                compose.waitUntil(20_000) { compose.onAllNodesWithTag("chat.composer.input").fetchSemanticsNodes().isNotEmpty() }
                val prompt = "Create an editable native form using render_ui, with a text field labeled Acceptance name, default Seed, and a button labeled Submit acceptance. When I submit it later, update this same card with a visible text saying A2UI-ANDROID-SUBMITTED followed by the submitted name. Do not simulate submission, use Markdown as a substitute, or access files. After creating the form, briefly answer and wait for my real action."
                compose.onNodeWithTag("chat.composer.input").performTextInput(prompt)
                compose.onNodeWithContentDescription(context.getString(R.string.chat_send)).performClick()
                }
                waitWithUiApprovals {
                    AndroidApplicationRuntime(context).use { app ->
                        val new = app.listConversations().filter { if (resumeConversation.isNotBlank()) it.id == resumeConversation else it.id !in before }
                        for (candidate in new) {
                            if (remoteConnection != null) {
                                val intent = app.readRemoteSubmissionIntent(candidate.id)
                                check(intent?.phase !in setOf(RemoteSubmissionPhase.Rejected, RemoteSubmissionPhase.Cancelled)) {
                                    "Source Host declined submission: ${intent?.phase}; conversation=${candidate.id}"
                                }
                                // A local draft exists before source admission; never query a nonexistent source conversation.
                                if (intent?.phase == RemoteSubmissionPhase.Admitted) admittedSourceConversations += candidate.id
                                // The app clears pending submission state after a successful terminal seal.
                                if (resumeConversation.isBlank() && candidate.id !in admittedSourceConversations) continue
                            }
                            val ready = surfaces(app,candidate.id).any { it.phase == UiSurfacePhase.Ready }
                            if (!ready) {
                                val transcript = app.execute(buildJsonObject {
                                    put("type", "load_conversation"); put("conversation_id", candidate.id)
                                }).jsonObject
                                val run = remoteConnection?.let { connection -> runBlocking {
                                    val discovered = decodeRemoteRunObservation(requireNotNull(remoteProvider).observationTransport(connection)(
                                        remoteObservationRequest(connection.expectedHostId, connection.clientId, candidate.id)),
                                        connection.expectedHostId, connection.clientId, candidate.id)
                                    discovered?.key?.also { remoteRunKeys[it.runId] = it }?.runId
                                } } ?: transcript.getValue("messages").jsonArray.map { it.jsonObject }
                                    .filter { it.getValue("role").jsonPrimitive.content == "assistant" }
                                    .mapNotNull { it["run_id"]?.jsonPrimitive?.contentOrNull }.lastOrNull()
                                if (run != null) check(successfulSeal(app, candidate.id, run) == null) {
                                    "Real model completed without publishing the requested UI Surface"
                                }
                            }
                        }
                        conversation = new.firstOrNull { surfaces(app,it.id).any { surface -> surface.phase == UiSurfacePhase.Ready } }?.id.orEmpty()
                        if (conversation.isNotEmpty()) {
                            val surface = surfaces(app,conversation).single { it.phase == UiSurfacePhase.Ready }
                            initialSeal = successfulSeal(app, conversation, surface.identity.owner.originating_run_id, surface.identity.owner.message_id)
                        }
                        initialSeal != null
                    }
                }
                val original = AndroidApplicationRuntime(context).use { surfaces(it,conversation).single { s -> s.phase == UiSurfacePhase.Ready } }
                initialRevision = original.revision
                File(context.filesDir, "a2ui-real-model-stage.txt").writeText("initial_sealed_materializing")
                materializeSurface(original)
                File(context.filesDir, "a2ui-real-model-stage.txt").writeText("editing")
                val originalOwner = hasAnyAncestor(SemanticsMatcher.expectValue(UiSurfaceIdentitySemantics, original.identity))
                compose.onNode(hasSetTextAction() and (hasText("Acceptance name") or hasAnyDescendant(hasText("Acceptance name"))) and originalOwner, useUnmergedTree = true)
                    .performScrollTo().performClick().performTextReplacement("Android验收")
                fun keyboardVisible(): Boolean {
                    var visible = false
                    scenario.onActivity { activity -> visible = androidx.core.view.ViewCompat.getRootWindowInsets(activity.window.decorView)
                        ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true }
                    return visible
                }
                compose.waitUntil(5_000) { keyboardVisible() }
                InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                compose.waitUntil(5_000) { !keyboardVisible() }
                compose.waitForIdle()
                val submit = SemanticsMatcher.expectValue(
                    androidx.compose.ui.semantics.SemanticsProperties.Role,
                    androidx.compose.ui.semantics.Role.Button,
                ) and hasAnyDescendant(hasText("Submit acceptance")) and originalOwner
                compose.onNode(submit, useUnmergedTree = true).performScrollTo()
                // Core validates a changed binding asynchronously. A Compose
                // click on a disabled action is a no-op, so wait for the real
                // product control to become enabled before submitting once.
                compose.waitUntil(20_000) {
                    compose.mainClock.advanceTimeBy(32)
                    compose.onAllNodes(submit and isEnabled(), useUnmergedTree = true)
                        .fetchSemanticsNodes().size == 1
                }
                File(context.filesDir, "a2ui-before-submit-semantics.txt")
                    .writeText(compose.onRoot(useUnmergedTree = true).printToString())
                compose.onNode(submit, useUnmergedTree = true)
                    .assertIsEnabled().performClick()
                File(context.filesDir, "a2ui-real-model-stage.txt").writeText("submitted_waiting_successor")
                waitWithUiApprovals {
                    AndroidApplicationRuntime(context).use { app ->
                        // Read the persisted run ownership, including already completed fast Runs.
                        val transcript = app.execute(buildJsonObject {
                            put("type", "load_conversation"); put("conversation_id", conversation)
                        }).jsonObject
                        val successorMessage = transcript.getValue("messages").jsonArray.map { it.jsonObject }
                            .lastOrNull { it.getValue("role").jsonPrimitive.content == "assistant" &&
                                it["run_id"]?.jsonPrimitive?.contentOrNull?.let { run -> run != original.identity.owner.originating_run_id } == true }
                        submittedRunId = successorMessage?.get("run_id")?.jsonPrimitive?.contentOrNull
                        updated = surfaces(app,conversation).firstOrNull { s ->
                            s.identity == original.identity && s.revision > original.revision && s.nodes.values.any { node ->
                                node.component == "Text" && node.props["text"]?.jsonPrimitive?.contentOrNull.orEmpty().let { text ->
                                    text.contains("A2UI-ANDROID-SUBMITTED") && text.contains("Android验收")
                                }
                            }
                        }
                        submittedSeal = submittedRunId?.let { successfulSeal(app, conversation, it, successorMessage?.get("id")?.jsonPrimitive?.contentOrNull) }
                        updated != null && submittedSeal != null
                    }
                }
                assertSurfaceTextVisible(requireNotNull(updated))
                // Recreate the real Activity, then navigate through the product's conversation list.
                scenario.recreate()
                compose.waitForIdle()
                val back = compose.onAllNodesWithTag("chat.conversation.back")
                if (back.fetchSemanticsNodes().isNotEmpty()) back.onFirst().performClick()
                else compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
                compose.waitUntil(20_000) { compose.onAllNodesWithTag("chat.conversation.$conversation").fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithTag("chat.conversation.$conversation").performScrollTo().performClick()
                
                assertSurfaceTextVisible(requireNotNull(updated))
                compose.onNodeWithTag("chat.conversation.back").performClick()
                compose.onNodeWithTag("chat.conversations.new").performClick()
                compose.waitUntil(20_000) { compose.onAllNodesWithTag("chat.composer.input").fetchSemanticsNodes().isNotEmpty() }
                compose.onAllNodes(SemanticsMatcher.keyIsDefined(UiSurfaceIdentitySemantics),useUnmergedTree=true).assertCountEquals(0)
            }
            val restored = AndroidApplicationRuntime(context).use { surfaces(it,conversation).single { s -> s.identity == requireNotNull(updated).identity } }
            assertEquals(requireNotNull(updated).revision, restored.revision)
            File(context.filesDir, "a2ui-real-model-result.json").writeText(JSONObject()
                .put("passed", true).put("conversation", conversation).put("revision", restored.revision)
                .put("initialRevision", initialRevision)
                .put("identity", JSONObject(kotlinx.serialization.json.Json.encodeToString(com.nola.data.local.UiSurfaceIdentity.serializer(), restored.identity)))
                .put("initialRun", safeRunEvidence(requireNotNull(initialSeal)))
                .put("submittedRun", safeRunEvidence(requireNotNull(submittedSeal)))
                .put("realProvider", true).put("realSourceHost",remoteConnection?.expectedHostId)
                .put("sourceAdmissions",org.json.JSONArray(remoteAdmissions.map { JSONObject(JsonObject(it.filterKeys { key -> key in setOf("identity","model_profile_id","model_profile_revision","admission_digest","runtime_config_digest") }).toString()) })).put("activityRecreated", true).put("newConversationIsolated",true).put("composeEditedAndSubmitted", true).toString())
        } finally {
            fixture.delete()
            if (remoteConnection != null) runBlocking { PreferenceDataStore(context,tokenStore).clearWebHostConfig() }
            else tokenStore.deleteLocalModelApiKey()
            remoteHttp?.close()
            context.getSharedPreferences("nola_secure_tokens", android.content.Context.MODE_PRIVATE).edit().commit()
        }
    }

    /** Read-only inspection of retained real-model evidence; never sends a model request. */
    @Test fun reopenPersistedSubmittedSurface() {
        check(InstrumentationRegistry.getArguments().getString("disposableA2uiFixture") == "true")
        val conversation = requireNotNull(InstrumentationRegistry.getArguments().getString("evidenceConversation"))
        val snapshot = AndroidApplicationRuntime(context).use { app ->
            surfaces(app,conversation).single { it.phase == UiSurfacePhase.Ready }
        }
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            compose.waitForIdle()
            compose.onNodeWithContentDescription(context.getString(R.string.tab_chat)).performClick()
            compose.waitUntil(20_000) { compose.onAllNodesWithTag("chat.conversation.$conversation").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithTag("chat.conversation.$conversation").performScrollTo().performClick()
            assertSurfaceTextVisible(snapshot)
            scenario.recreate()
            compose.waitForIdle()
            assertSurfaceTextVisible(snapshot)
        }
    }

    private fun materializeSurface(snapshot: UiSnapshot) {
        // LazyColumn disposes the originating message while following the successor response.
        // Materialize that exact owner item before querying its card, rather than accepting prose.
        compose.waitUntil(20_000) {
            compose.onAllNodesWithTag("chat-transcript-list").fetchSemanticsNodes().any {
                it.config[ChatTranscriptMessageIds].contains(snapshot.identity.owner.message_id)
            }
        }
        // Match a person reviewing earlier content: a real drag exits follow-latest mode.
        // Programmatic scroll alone is intentionally superseded by streaming follow behavior.
        compose.onNodeWithTag("chat-transcript-list").performTouchInput { swipeDown() }
        compose.onNodeWithTag("chat-transcript-list").performScrollToKey(ChatTimelineMotionPolicy.itemKey(snapshot.identity.owner.conversation_id, snapshot.identity.owner.message_id))
        compose.waitUntil(20_000) {
            compose.onAllNodes(SemanticsMatcher.expectValue(UiSurfaceIdentitySemantics, snapshot.identity), useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun assertSurfaceTextVisible(snapshot: UiSnapshot) {
        val text = snapshot.nodes.values.first { node ->
            node.component == "Text" && node.props["text"]?.jsonPrimitive?.contentOrNull.orEmpty().let {
                it.contains("A2UI-ANDROID-SUBMITTED") && it.contains("Android验收")
            }
        }.props.getValue("text").jsonPrimitive.content
        materializeSurface(snapshot)
        val matcher = hasText(text) and hasAnyAncestor(
            SemanticsMatcher.expectValue(UiSurfaceIdentitySemantics, snapshot.identity))
        try {
            compose.waitUntil(20_000) { compose.onAllNodes(matcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        } catch (failure: Exception) {
            File(context.filesDir, "a2ui-reopen-semantics.txt").writeText(compose.onAllNodes(isRoot(),useUnmergedTree=true).onFirst().printToString())
            throw failure
        }
        compose.onAllNodes(matcher, useUnmergedTree = true).onFirst().performScrollTo().assertIsDisplayed()
    }

    /** Absence of activeRun is not completion: require the durable successful result seal. */
    private fun successfulSeal(app: AndroidApplicationRuntime, conversation: String, run: String, sourceMessageId: String? = null): JsonObject? {
        val value = remoteConnection?.let { connection -> runBlocking {
            val transport = requireNotNull(remoteProvider).observationTransport(connection)
            val known = sourceMessageId?.let { RemoteRunKey(conversation, run, it) } ?: remoteRunKeys[run]
            val observed = decodeRemoteRunObservation(transport(
                remoteObservationRequest(connection.expectedHostId, connection.clientId, conversation, known)),
                connection.expectedHostId, connection.clientId, conversation, known)
            observed?.key?.let { remoteRunKeys[it.runId] = it }
            // A local optimistic message is not a source-Host Run identity.
            if (observed?.key?.runId != run) return@runBlocking JsonNull
            observed?.admission?.let { admission -> if (remoteAdmissions.none { it == admission }) remoteAdmissions += admission }
            observed?.terminalSeal ?: JsonNull
        } } ?: app.execute(buildJsonObject {
            put("type", "local_assistant_run_terminal_seal")
            put("conversation_id", conversation)
            put("run_id", run)
        })
        if (value is JsonNull) return null
        val seal = value.jsonObject
        val result = seal.getValue("result").jsonObject
        check(result.getValue("run_id").jsonPrimitive.content == run)
        check(result.getValue("conversation_id").jsonPrimitive.content == conversation)
        val kind = result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content
        check(kind == "answered" || kind == "action_completed") { "Real model Run ended unsuccessfully: $kind" }
        return seal
    }

    private fun safeRunEvidence(seal: JsonObject): JSONObject = JSONObject()
        .put("runId", seal.getValue("result").jsonObject.getValue("run_id").jsonPrimitive.content)
        .put("resultKind", seal.getValue("result").jsonObject.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content)
        .put("sealDigest", seal.getValue("seal_digest").jsonPrimitive.content)
        .put("completedAtMs", seal.getValue("completed_at_ms").jsonPrimitive.long)

    private fun waitWithUiApprovals(condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 240_000
        while (SystemClock.elapsedRealtime() < deadline) {
            compose.waitForIdle()
            if (condition()) return
            val pending = compose.onAllNodesWithTag("chat-pending-permission")
            if (pending.fetchSemanticsNodes().isNotEmpty()) pending.onFirst().performClick()
            val allow = compose.onAllNodesWithText(context.getString(R.string.permission_action_allow_once))
            if (allow.fetchSemanticsNodes().isNotEmpty()) {
                // This scenario authorizes only UI publication, never another tool.
                val toolLabel = com.nola.ui.components.hostToolDisplayNameStringRes("render_ui")?.let(context::getString)
                    ?: context.getString(R.string.chat_host_tool_name_unknown_format, "render_ui")
                val uiTitle = context.getString(R.string.permission_required_title, toolLabel)
                val uiOnly = compose.onAllNodesWithText(uiTitle, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
                check(uiOnly) { "Unexpected non-UI permission request; no approval was issued" }
                allow.onFirst().performScrollTo().performClick()
            }
            SystemClock.sleep(250)
        }
        fail("Real provider did not complete the required A2UI stage within 240 seconds")
    }
}
