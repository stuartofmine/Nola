// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.MainActivity
import com.nola.R
import com.nola.ui.WarmPaletteExpectations
import com.nola.ui.assertWarmThemeSurface
import com.nola.ui.colorsMatchWithinTolerance
import com.nola.ui.dominantInteriorColor
import com.nola.ui.toHexRgb
import com.nola.ui.wcagContrastRatio
import com.nola.ui.screen.onboarding.OnboardingContent
import com.nola.ui.screen.onboarding.OnboardingUiState
import com.nola.ui.theme.NolaTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlin.math.roundToInt
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** MainActivity navigation at phone, compact-side and wide-side sizes in a disposable user. No inference. */
@RunWith(AndroidJUnit4::class)
class WorkspacePageMatrixTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val primaryNavigation = hasTestTag("navigation.bottom") or hasTestTag("navigation.side.compact") or hasTestTag("navigation.side.expanded")
    private fun tab(label: Int) {
        val destination = (hasText(context.getString(label)) or hasContentDescription(context.getString(label))) and hasAnyAncestor(primaryNavigation)
        compose.onNode(destination).performClick()
        compose.onNode(destination and isSelected()).assertIsDisplayed()
    }
    private fun captureSystemWindow(name: String, packageName: String) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        compose.waitUntil(10_000) { automation.rootInActiveWindow?.packageName == packageName }
        // The accessibility window precedes the first compositor frames on a cold launch.
        automation.waitForIdle(500, 5_000)
        android.os.SystemClock.sleep(350)
        val file = File(context.filesDir, "page-matrix/$name.png")
        file.parentFile?.mkdirs()
        val screenshot = automation.executeShellCommand("screencap -p")
        val screen = android.os.ParcelFileDescriptor.AutoCloseInputStream(screenshot).use {
            checkNotNull(android.graphics.BitmapFactory.decodeStream(it)) { "Missing screenshot for $name" }
        }
        try {
            check(automation.rootInActiveWindow?.packageName == packageName) { "Window changed during $name" }
            file.outputStream().use { check(screen.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally {
            screen.recycle()
        }
    }

    private fun capture(name: String, darkTheme: Boolean? = null) {
        compose.mainClock.advanceTimeBy(500)
        compose.waitForIdle()
        // Android's native ripple runs on RenderThread, outside Compose's
        // virtual clock. Capture the settled page rather than a recent tap.
        android.os.SystemClock.sleep(400)
        compose.waitForIdle()
        if (compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty()) {
            compose.onAllNodes(isSelected() and hasAnyAncestor(primaryNavigation))
                .assertCountEquals(1)
        }
        // Capture the frame shown by the emulator's compositor. Compose's window
        // PixelCopy can time out on an idle page with no new buffers to submit.
        // A dialog adds a second Compose root. Crop the full workspace window
        // so the screenshot includes both the dialog and its surroundings.
        val root = compose.onAllNodes(isRoot()).fetchSemanticsNodes().maxBy { it.size.width * it.size.height }
        val (position, size) = compose.runOnIdle { root.positionOnScreen to root.size }
        val screen = checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()) {
            "The compositor did not return a screenshot for $name"
        }
        val cropped = Bitmap.createBitmap(screen, position.x.roundToInt(), position.y.roundToInt(), size.width, size.height)
        if (cropped !== screen) screen.recycle()
        val image = cropped.asImageBitmap()
        if (darkTheme != null) {
            val pixels = image.assertWarmThemeSurface(darkTheme = darkTheme, label = "WorkspacePageMatrix($name)")
            val surfaceSample = pixels.dominantInteriorColor()
            val textNodes = compose.onAllNodes(
                SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult),
                useUnmergedTree = true,
            ).fetchSemanticsNodes()
            val results = mutableListOf<TextLayoutResult>()
            for (node in textNodes) {
                results.clear()
                node.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(results)
                for (layout in results) {
                    val color = layout.layoutInput.style.color
                    if (color == Color.Unspecified || color.alpha < 0.9f) continue
                    if (darkTheme) {
                        org.junit.Assert.assertFalse(
                            "WorkspacePageMatrix($name) rendered near-black text ${color.toHexRgb()} in Dark mode",
                            colorsMatchWithinTolerance(color, Color.Black, 10),
                        )
                    }
                    if (colorsMatchWithinTolerance(color, if (darkTheme) WarmPaletteExpectations.DarkText else WarmPaletteExpectations.LightText, 8) ||
                        colorsMatchWithinTolerance(color, if (darkTheme) WarmPaletteExpectations.DarkSecondaryText else WarmPaletteExpectations.LightSecondaryText, 8)
                    ) {
                        val ratio = wcagContrastRatio(color, surfaceSample)
                        org.junit.Assert.assertTrue(
                            "WorkspacePageMatrix($name) text ${color.toHexRgb()} vs ${surfaceSample.toHexRgb()} contrast $ratio < 4.5",
                            ratio >= 4.5f,
                        )
                    }
                }
            }
        }
        val directory = File(context.filesDir, "page-matrix").apply { mkdirs() }
        File(directory, "$name.png").outputStream().use {
            check(image.asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it))
        }
        cropped.recycle()
    }
    private fun back(activity: ActivityScenario<MainActivity>) {
        compose.waitForIdle()
        activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
    }

    private fun enterWorkspace() {
        val skip = hasText(context.getString(R.string.onboarding_guide_action_later))
        compose.waitUntil(15_000) {
            compose.onAllNodes(primaryNavigation or skip).fetchSemanticsNodes().isNotEmpty()
        }
        if (compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isEmpty()) {
            compose.onNode(skip).performClick()
        }
        compose.waitUntil(15_000) { compose.onAllNodes(primaryNavigation).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun allPrimaryAndManagementPagesOpenAndReturnInBothThemes() {
        requireIsolatedProductTestUser(context)
        // Verify OnboardingScreen renders Warm Paper & Warm Ink surfaces across both themes
        // without mutating product onboarding persistence.
        for ((theme, isDark) in listOf("light" to false, "dark" to true)) {
            ActivityScenario.launch(ComponentActivity::class.java).use { host ->
                host.onActivity { activity ->
                    activity.setContent {
                        NolaTheme(darkTheme = isDark) {
                            OnboardingContent(uiState = OnboardingUiState(), onComplete = {})
                        }
                    }
                }
                compose.waitForIdle()
                compose.onNodeWithText(context.getString(R.string.onboarding_choose_model)).assertIsDisplayed()
                compose.onNodeWithText(context.getString(R.string.onboarding_guide_action_later)).assertIsDisplayed()
                capture("onboarding-$theme", darkTheme = isDark)
            }
        }
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            for ((theme, label) in listOf("light" to R.string.appearance_light, "dark" to R.string.appearance_dark)) {
                val isDark = theme == "dark"
                tab(R.string.tab_settings)
                compose.onNodeWithText(context.getString(R.string.settings_general)).performClick()
                compose.onNodeWithText(context.getString(label)).performClick()
                compose.waitUntil(10_000) {
                    compose.onAllNodes(hasText(context.getString(label)) and isSelected()).fetchSemanticsNodes().isNotEmpty()
                }
                compose.mainClock.advanceTimeBy(1_000)
                compose.waitForIdle()
                capture("settings-appearance-$theme", darkTheme = isDark)
                back(activity)
                for (section in listOf(R.string.settings_general, R.string.settings_privacy, R.string.models_title,
                    R.string.chat_experts, R.string.settings_capabilities)) {
                    tab(R.string.tab_settings)
                    compose.onNodeWithText(context.getString(section)).performScrollTo().performClick()
                    when (section) {
                        R.string.settings_general -> compose.onNodeWithText(context.getString(R.string.appearance_system)).assertIsDisplayed()
                        R.string.settings_privacy -> compose.onNodeWithText(context.getString(R.string.settings_local_data_size)).assertIsDisplayed()
                        R.string.models_title -> compose.onNodeWithTag("models.refresh").assertIsDisplayed()
                        R.string.chat_experts -> compose.onNodeWithTag("expert.create").assertIsDisplayed()
                        R.string.settings_capabilities -> compose.onNodeWithTag("intelligence.capabilities.skills").assertIsSelected()
                    }
                    capture("settings-${context.resources.getResourceEntryName(section)}-$theme", darkTheme = isDark)
                    if (section == R.string.settings_general) {
                        compose.onNodeWithTag("settings_notification_center").performScrollTo().performClick()
                        back(activity)
                        capture("general-return-$theme", darkTheme = isDark)
                        compose.onNodeWithTag("settings_notification_center").assertIsDisplayed()
                        compose.onNodeWithText(context.getString(R.string.software_update_check)).performScrollTo().assertIsDisplayed()
                        capture("general-about-$theme", darkTheme = isDark)
                    }
                    back(activity)
                    compose.onNodeWithText(context.getString(R.string.settings_general)).assertIsDisplayed()
                }
                tab(R.string.tab_information)
                capture("information-$theme", darkTheme = isDark)
                val overview = compose.onAllNodesWithTag("information.overview").fetchSemanticsNodes().isNotEmpty()
                for (page in if (overview) listOf(4, 1, 0) else listOf(4, 1, 0, 2, 3)) {
                    compose.onNodeWithTag(if (overview) "information.category.$page" else "workspace.section.$page")
                        .performScrollTo().performClick()
                    capture("information-$page-$theme", darkTheme = isDark)
                    if (overview) {
                        back(activity)
                        compose.onNodeWithTag("information.overview").assertIsDisplayed()
                    }
                }
                for (destination in listOf(R.string.tab_chat, R.string.tab_activity, R.string.tab_settings)) {
                    tab(destination)
                    if (destination == R.string.tab_activity) {
                        val layouts = mutableListOf<TextLayoutResult>()
                        val label = hasText(context.getString(R.string.activity_search_title)) and
                            SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult)
                        compose.onNode(label, useUnmergedTree = true)
                            .performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
                        val layout = layouts.single()
                        org.junit.Assert.assertFalse("Activity search placeholder must fit on one line", layout.isLineEllipsized(0))
                    }
                    capture("primary-${context.resources.getResourceEntryName(destination)}-$theme", darkTheme = isDark)
                }
            }
        }
    }

    @Test fun settingsEditorsOpenAndCancelWithoutLosingTheirDestination() {
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            tab(R.string.tab_settings)
            fun click(label: Int) = compose.onNode(hasText(context.getString(label)) and hasClickAction())
                .performScrollTo().performClick()
            click(R.string.settings_privacy)
            for (label in listOf(R.string.settings_export_data, R.string.settings_clear_all_data)) {
                click(label)
                compose.onNodeWithText(context.getString(R.string.common_cancel)).assertIsDisplayed()
                capture("privacy-" + context.resources.getResourceEntryName(label))
                compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
                compose.onNodeWithText(context.getString(R.string.settings_privacy)).assertIsDisplayed()
            }
            back(activity)
            click(R.string.settings_capabilities)
            click(R.string.chat_create_template_skill)
            capture("skills-new-template")
            compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
            compose.onNodeWithTag("skill.editor.name").assertDoesNotExist()
            val mcpTab = compose.onNodeWithTag("intelligence.capabilities.mcp")
            mcpTab.performClick().assertIsSelected()
            click(R.string.chat_add_mcp_endpoint)
            compose.onNodeWithTag("mcp.editor.endpoint").assertIsDisplayed()
            capture("mcp-new-endpoint")
            compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
            compose.onNodeWithTag("mcp.editor.endpoint").assertDoesNotExist()
            val pluginsTab = compose.onNodeWithTag("intelligence.capabilities.plugins")
            pluginsTab.performClick().assertIsSelected()
            compose.onNodeWithTag("intelligence.harness.plugins").performClick()
            compose.onNodeWithText(context.getString(R.string.models_web_host_paste_setup)).assertIsDisplayed()
            capture("plugins-pair-host")
            compose.onNodeWithText(context.getString(R.string.common_cancel)).performClick()
            compose.onNodeWithText(context.getString(R.string.models_web_host_paste_setup)).assertDoesNotExist()
            pluginsTab.assertIsSelected()
            back(activity)
            click(R.string.scheduled_title)
            capture("scheduled-actions")
            click(R.string.scheduled_action_editor_new)
            compose.onNodeWithTag("scheduled-task-instruction").assertExists()
            capture("scheduled-editor")
            // The sheet owns a separate window; send Back through Android.
            InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            compose.waitForIdle()
            compose.onNodeWithTag("scheduled-task-instruction").assertDoesNotExist()
            compose.onNodeWithTag("scheduled.create").assertIsDisplayed()
            back(activity)
            compose.onNodeWithText(context.getString(R.string.settings_general)).assertExists()
        }
    }

    @Test fun createRenameAndDeleteSourceThroughRealWorkspace() {
        requireIsolatedProductTestUser(context)
        val name = "Audit source " + java.util.UUID.randomUUID().toString().take(8)
        com.nola.data.repository.SourceFixtureServer("""{"reading": 25}""").use { source ->
            ActivityScenario.launch(MainActivity::class.java).use {
                enterWorkspace()
                tab(R.string.tab_information)
                compose.onNodeWithTag("information.category.0").assertTextContains(context.getString(R.string.data_sources_screen_title)).performClick()
                compose.onNodeWithTag("data.sources.add").performScrollTo().performClick()
                val save = compose.onNodeWithTag("data.sources.editor.test_save")
                save.assertIsNotEnabled()
                compose.onNodeWithTag("data.sources.editor.name").performTextReplacement(name)
                compose.onNodeWithTag("data.sources.editor.location").performTextReplacement(source.url)
                save.performClick()
                fun openSource(label: String) {
                    compose.waitUntil(15_000) {
                        compose.onAllNodesWithTag("data.sources.editor").fetchSemanticsNodes().isEmpty()
                    }
                    val match = hasText(label) and hasClickAction()
                    compose.onNodeWithTag("data.sources").performScrollToNode(match)
                    compose.onNode(match).performClick()
                }
                openSource(name)
                compose.onNodeWithTag("data.sources.editor.name").performTextReplacement("$name renamed")
                save.performClick()
                openSource("$name renamed")
                capture("source-saved-renamed")
                compose.onNodeWithText(context.getString(R.string.common_delete)).performClick()
                compose.onNodeWithText(context.getString(R.string.data_sources_delete_confirm_title)).assertIsDisplayed()
                compose.onAllNodesWithText(context.getString(R.string.common_delete)).onLast().performClick()
                compose.waitUntil(10_000) {
                    compose.onAllNodesWithTag("data.sources.editor").fetchSemanticsNodes().isEmpty()
                }
                compose.waitUntil(15_000) { compose.onAllNodesWithText("$name renamed").fetchSemanticsNodes().isEmpty() }
            }
        }
    }

    @Test fun resetLegacyExpertProfileKeepsOtherSettings() {
        requireIsolatedProductTestUser(context)
        val id = java.util.UUID.randomUUID().toString()
        val key = "configured:$id"
        val name = "Audit old expert " + id.take(8)
        com.nola.data.local.AndroidApplicationRuntime(context).use { runtime ->
            runtime.upsertModelProfile(com.nola.data.local.CanonicalModelProfile(
                id, name, "openai-compatible", id, "remote",
                """{"baseURL":"http://127.0.0.1:9/v1","wireProtocol":"responses","executionCapabilities":{"toolCalls":true}}""",
                "{}", false, true, 0, 1, 1,
            ))
            val rooms = com.nola.data.repository.AgentRoomRepository(runtime)
            val defaults = Json.parseToJsonElement("""{"modelSelectionID":"$key","modelOverrides":{"$key":{"temperature":0.7,"topP":0.9},"configured:other":{"temperature":0.25}}}""").jsonObject
            val created = rooms.createAgent(name, defaults, rooms.directory(), "Review", id).agents.single { it.id == id }
            try {
                ActivityScenario.launch(MainActivity::class.java).use {
                    enterWorkspace()
                    tab(R.string.tab_settings)
                    compose.onNodeWithText(context.getString(R.string.chat_experts)).performScrollTo().performClick()
                    val row = hasText(name) and hasClickAction()
                    compose.onNodeWithTag("expert.directory.list").performScrollToNode(row)
                    compose.onNode(row).performClick()
                    val reset = compose.onNodeWithTag("expert.editor.reset-model-parameters")
                    reset.performScrollTo().assertIsEnabled().performClick()
                    reset.assertIsNotEnabled()
                    capture("expert-use-model-defaults")
                    compose.onNodeWithTag("expert.editor.save").performScrollTo().performClick()
                    compose.waitUntil(10_000) { compose.onAllNodesWithTag("expert.create").fetchSemanticsNodes().isNotEmpty() }
                    val result = rooms.directory().agents.single { it.id == created.id }
                    check(result.defaults["modelSelectionID"] == defaults["modelSelectionID"])
                    check(result.defaults.getValue("modelOverrides").jsonObject.keys == setOf("configured:other"))
                }
            } finally {
                val snapshot = rooms.directory()
                val current = snapshot.agents.single { it.id == created.id }
                rooms.updateAgent(current.copy(availability = "archived"), current.name, current.description, snapshot)
                runtime.deleteModelProfile(id, System.currentTimeMillis())
            }
        }
    }

    @Test fun createTemplatedExpertRenameAndArchiveInDirectory() {
        requireIsolatedProductTestUser(context)
        val name = "Audit expert " + java.util.UUID.randomUUID().toString().take(8)
        ActivityScenario.launch(MainActivity::class.java).use {
            enterWorkspace()
            tab(R.string.tab_settings)
            compose.onNodeWithText(context.getString(R.string.chat_experts)).performScrollTo().performClick()
            compose.onNodeWithText(context.getString(R.string.expert_manage_source_local)).assertIsDisplayed()
            val create = compose.onNodeWithTag("expert.create")
            compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag("expert.create") and isEnabled()).fetchSemanticsNodes().isNotEmpty() }
            create.performClick()
            compose.onNodeWithTag("expert.editor.name").performTextReplacement(name)
            compose.onNodeWithTag("expert.editor.description").performTextReplacement("Review the tests")
            compose.onNodeWithTag("expert.editor.template").performScrollTo().performClick()
            val template = SemanticsMatcher("built-in expert role") {
                it.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)?.startsWith("expert.template.") == true
            }
            compose.onAllNodes(template).onFirst().performClick()
            val preview = compose.onNodeWithTag("expert.template.preview")
            preview.performScrollTo().performClick()
            compose.onNodeWithText(context.getString(R.string.expert_role_tools)).performScrollTo().assertIsDisplayed()
            compose.onNodeWithText(context.getString(R.string.expert_role_skills)).performScrollTo().assertIsDisplayed()
            capture("expert-template")
            preview.performScrollTo().performClick()
            compose.onNodeWithTag("expert.editor.effort").performScrollTo().performClick()
            compose.onNodeWithTag("expert.effort.automatic").performClick()
            fun saveEditor() {
                compose.onNodeWithText(context.getString(R.string.common_save)).performScrollTo().performClick()
                compose.waitUntil(15_000) { compose.onAllNodesWithTag("expert.create").fetchSemanticsNodes().isNotEmpty() }
            }
            fun row(label: String): SemanticsNodeInteraction {
                val match = hasText(label) and hasClickAction()
                compose.onNodeWithTag("expert.directory.list").performScrollToNode(match)
                return compose.onNode(match)
            }
            saveEditor()
            row(name).performClick()
            compose.onNodeWithTag("expert.editor.name").performTextReplacement("$name revised")
            saveEditor()
            val item = row("$name revised")
            val id = item.fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.TestTag].removePrefix("expert.row.")
            compose.onNodeWithTag("expert.actions.$id").performClick()
            compose.onNodeWithTag("expert.availability.$id").performClick()
            compose.waitUntil(10_000) {
                compose.onAllNodes(hasTestTag("expert.row.$id") and hasText(context.getString(R.string.expert_archived))).fetchSemanticsNodes().isNotEmpty()
            }
            capture("expert-archived")
        }
    }

    @Test fun activityFiltersSearchAndPlansPreserveTheirOrigin() {
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            tab(R.string.tab_activity)
            fun chip(label: Int) = compose.onNode(hasText(context.getString(label)) and hasClickAction())
            chip(R.string.activity_agent).performClick().assertIsSelected()
            chip(R.string.activity_plan).performClick().assertIsSelected()
            chip(R.string.activity_needs_response).performClick().assertIsSelected()
            val query = "audit-empty-" + java.util.UUID.randomUUID().toString().take(8)
            val search = compose.onNode(hasSetTextAction())
            search.performTextReplacement(query)
            search.performImeAction()
            compose.waitUntil(15_000) {
                compose.onAllNodesWithText(context.getString(R.string.activity_empty_filter)).fetchSemanticsNodes().isNotEmpty()
            }
            capture("activity-filtered-search")
            compose.onNodeWithContentDescription(context.getString(R.string.common_refresh)).performClick()
            compose.onNodeWithContentDescription(context.getString(R.string.scheduled_title)).performClick()
            compose.onNodeWithTag("scheduled.create").assertIsDisplayed()
            compose.onNode((hasText(context.getString(R.string.tab_activity)) or hasContentDescription(context.getString(R.string.tab_activity))) and
                hasAnyAncestor(primaryNavigation) and isSelected()).assertIsDisplayed()
            back(activity)
            search.assertTextContains(query)
            chip(R.string.activity_plan).assertIsSelected()
            chip(R.string.activity_needs_response).assertIsSelected()
        }
    }

    @Test fun manageSkillAndMcpThroughSettings() {
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use {
            enterWorkspace()
            tab(R.string.tab_settings)
            compose.onNodeWithText(context.getString(R.string.settings_capabilities)).performScrollTo().performClick()
            fun editField(tag: String, value: String) {
                compose.waitUntil(10_000) { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() }
                compose.onNodeWithTag(tag).performTextReplacement(value)
            }
            fun click(label: Int) = compose.onNode(hasText(context.getString(label)) and hasClickAction()).performClick()
            val suffix = java.util.UUID.randomUUID().toString().take(8)
            for (skill in listOf(true, false)) {
                if (!skill) compose.onNodeWithTag("intelligence.capabilities.mcp").performClick().assertIsSelected()
                val name = if (skill) "audit-skill-$suffix" else "Audit MCP $suffix"
                val nameTag = if (skill) "skill.editor.name" else "mcp.editor.name"
                click(if (skill) R.string.chat_create_template_skill else R.string.chat_add_mcp_endpoint)
                editField(nameTag, name)
                if (skill) editField("skill.editor.content", "Return the current test document title.")
                else {
                    editField("mcp.editor.endpoint", "invalid-url")
                    compose.onNodeWithText(context.getString(R.string.common_save)).assertIsNotEnabled()
                    editField("mcp.editor.endpoint", "http://127.0.0.1:18080/mcp")
                }
                click(R.string.common_save)
                fun tile(label: String): SemanticsMatcher = hasText(label) and hasClickAction()
                fun waitForTile(label: String) {
                    compose.waitUntil(15_000) { compose.onAllNodes(tile(label)).fetchSemanticsNodes().isNotEmpty() }
                }
                waitForTile(name)
                compose.onNode(tile(name)).performScrollTo().performClick()
                val revised = "$name-updated"
                editField(nameTag, revised)
                click(R.string.common_save)
                waitForTile(revised)
                val toggler = isToggleable() and hasAnyAncestor(tile(revised))
                val wasOn = compose.onAllNodes(toggler and isOn()).fetchSemanticsNodes().isNotEmpty()
                compose.onNode(toggler).performScrollTo().assertIsEnabled().performClick()
                compose.waitUntil(15_000) {
                    compose.onAllNodes(toggler and if (wasOn) isOff() else isOn()).fetchSemanticsNodes().isNotEmpty()
                }
                capture(if (skill) "skill-saved-toggled" else "mcp-saved-toggled")
                val more = hasContentDescription(context.getString(R.string.intelligence_more)) and hasAnyAncestor(tile(revised))
                compose.onNode(more).performScrollTo().performClick()
                click(R.string.common_delete)
                compose.onNode(hasText(context.getString(R.string.common_delete)) and hasClickAction() and hasAnyAncestor(isDialog())).performClick()
                compose.waitUntil(15_000) { compose.onAllNodes(tile(revised)).fetchSemanticsNodes().isEmpty() }
            }
        }
    }

    @Test fun createPauseEditAndRemoveDailySchedule() {
        requireIsolatedProductTestUser(context)
        if (android.os.Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(
                android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            // Android suppresses repeat permission prompts after a second denial.
            val ownedUser = context.applicationInfo.uid / 100_000
            val reset = "pm clear-permission-flags --user $ownedUser ${context.packageName} android.permission.POST_NOTIFICATIONS user-set user-fixed"
            val descriptor = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(reset)
            android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).bufferedReader().use { check(it.readText().isBlank()) }
        }
        val name = "Audit schedule " + java.util.UUID.randomUUID().toString().take(8)
        com.nola.data.repository.SourceFixtureServer("""{"status":"ok"}""").use { source ->
            ActivityScenario.launch(MainActivity::class.java).use {
                enterWorkspace()
                tab(R.string.tab_settings)
                compose.onNodeWithText(context.getString(R.string.scheduled_title)).performScrollTo().performClick()
                compose.onNodeWithTag("scheduled.create").performClick()
                val editor = compose.onNodeWithTag("scheduled.editor")
                fun field(matcher: SemanticsMatcher): SemanticsNodeInteraction {
                    editor.performScrollToNode(matcher)
                    return compose.onNode(matcher and hasAnyAncestor(hasTestTag("scheduled.editor")))
                }
                val nameField = hasText(context.getString(R.string.scheduled_name)) and hasSetTextAction()
                field(nameField).performTextReplacement(name)
                field(hasTestTag("scheduled-task-instruction")).performTextReplacement("Read the local fixture")
                field(hasText(context.getString(R.string.scheduled_url)) and hasSetTextAction()).performTextReplacement(source.url)
                field(hasText(context.getString(R.string.scheduled_daily)) and hasClickAction()).performClick()
                field(hasTestTag("schedule-time")).performClick()
                compose.onNodeWithTag("schedule-picker-confirm").performClick()
                fun save(enableNotifications: Boolean) {
                    val requestsNotifications = enableNotifications && android.os.Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(
                        android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED
                    field(hasTestTag("schedule-save")).assertIsEnabled().performClick()
                    if (requestsNotifications) {
                        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
                        val info = automation.serviceInfo
                        info.flags = info.flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
                        automation.serviceInfo = info
                        compose.waitUntil(10_000) {
                            val window = automation.rootInActiveWindow
                            val pkg = window?.packageName?.toString().orEmpty()
                            val button = if (window != null && pkg.endsWith(".permissioncontroller")) {
                                listOf(pkg, "com.android.permissioncontroller").firstNotNullOfOrNull {
                                    window.findAccessibilityNodeInfosByViewId("$it:id/permission_deny_button").singleOrNull()
                                }
                            } else null
                            button?.let {
                                captureSystemWindow("schedule-notification-permission", it.packageName.toString())
                                button.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)
                            } == true
                        }
                    }
                    compose.waitUntil(20_000) {
                        runCatching { compose.onAllNodesWithTag("scheduled.create").fetchSemanticsNodes().isNotEmpty() &&
                            compose.onAllNodesWithTag("scheduled.editor").fetchSemanticsNodes().isEmpty() }.getOrDefault(false)
                    }
                    if (requestsNotifications) {
                        val settings = compose.onNodeWithTag("scheduled.notification-settings")
                        settings.performScrollTo().assertIsDisplayed()
                        capture("scheduled-notification-denied")
                        settings.performClick()
                        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
                        compose.waitUntil(10_000) {
                            val window = automation.rootInActiveWindow
                            window?.packageName == "com.android.settings" &&
                                window.findAccessibilityNodeInfosByText(context.applicationInfo.loadLabel(context.packageManager).toString())
                                    .any { it.isVisibleToUser }
                        }
                        captureSystemWindow("schedule-notification-settings", "com.android.settings")
                        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                        compose.waitUntil(10_000) {
                            runCatching { settings.fetchSemanticsNode() }.isSuccess
                        }
                    }
                }
                save(enableNotifications = true)
                val title = compose.onNode(hasText(context.getString(R.string.scheduled_title)) and
                    SemanticsMatcher.keyIsDefined(SemanticsActions.GetTextLayoutResult), useUnmergedTree = true)
                compose.waitUntil(10_000) {
                    val layout = mutableListOf<TextLayoutResult>()
                    title.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layout) }
                    layout.singleOrNull()?.let { it.lineCount == 1 && !it.isLineEllipsized(0) } == true
                }
                val list = compose.onNode(hasScrollToIndexAction())
                list.performScrollToNode(hasText(name))
                val card = SemanticsMatcher("saved schedule") { node ->
                    node.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)?.startsWith("scheduled.card.") == true
                } and hasAnyDescendant(hasText(name))
                val id = compose.onNode(card).fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsProperties.TestTag].removePrefix("scheduled.card.")
                list.performScrollToNode(hasTestTag("scheduled.toggle.$id"))
                compose.onNodeWithTag("scheduled.toggle.$id").assertIsOn().performClick()
                // Pausing can move the card below the current LazyColumn viewport.
                compose.waitUntil(15_000) {
                    list.performScrollToNode(hasTestTag("scheduled.toggle.$id"))
                    compose.onAllNodes(hasTestTag("scheduled.toggle.$id") and isOff()).fetchSemanticsNodes().isNotEmpty()
                }
                list.performScrollToNode(hasTestTag("scheduled.edit.$id"))
                compose.onNodeWithTag("scheduled.edit.$id").performClick()
                field(nameField).performTextReplacement("$name revised")
                field(nameField).performImeAction()
                save(enableNotifications = false)
                list.performScrollToNode(hasText("$name revised"))
                capture("scheduled-paused-renamed")
                val menu = hasContentDescription(context.getString(R.string.scheduled_more_options)) and hasAnyAncestor(hasTestTag("scheduled.card.$id"))
                compose.onNode(menu).performClick()
                val delete = hasText(context.getString(R.string.common_delete)) and hasClickAction()
                compose.onNode(delete).performClick()
                compose.onNode(delete and hasAnyAncestor(isDialog())).performClick()
                compose.waitUntil(15_000) { compose.onAllNodesWithTag("scheduled.card.$id").fetchSemanticsNodes().isEmpty() }
            }
        }
    }

    @Test fun saveActivateAndRemoveApiModelInSettings() {
        requireIsolatedProductTestUser(context)
        val model = "audit-model-" + java.util.UUID.randomUUID().toString().take(8)
        ActivityScenario.launch(MainActivity::class.java).use {
            enterWorkspace()
            tab(R.string.tab_settings)
            compose.onNodeWithText(context.getString(R.string.models_title)).performScrollTo().performClick()
            compose.onNodeWithTag("model.setup.endpoint").performScrollTo().performTextReplacement("http://127.0.0.1:8080/v1")
            compose.onNodeWithTag("model.setup.modelID").performScrollTo().performTextReplacement(model)
            compose.onNodeWithText(context.getString(R.string.models_protocol_responses)).performScrollTo().performClick()
            compose.onNodeWithTag("model.setup.save").performScrollTo().assertIsEnabled().performClick()
            val row = hasTestTag("models.row.$model")
            compose.waitUntil(15_000) { compose.onAllNodes(row).fetchSemanticsNodes().isNotEmpty() }
            compose.onNode(row).performScrollTo().assertIsDisplayed()
            capture("model-api-activated")
            val delete = context.getString(R.string.common_delete)
            compose.onNode(hasContentDescription(delete) and hasAnyAncestor(row)).performClick()
            compose.onNode(hasText(delete) and hasClickAction()).performClick()
            compose.waitUntil(10_000) { compose.onAllNodes(row).fetchSemanticsNodes().isEmpty() }
        }
    }

    @Test fun generalScrollSurvivesNotificationCenterRoundTrip() {
        requireIsolatedProductTestUser(context)
        ActivityScenario.launch(MainActivity::class.java).use { activity ->
            enterWorkspace()
            tab(R.string.tab_settings)
            compose.onNodeWithText(context.getString(R.string.settings_general)).performClick()
            compose.onNodeWithText(context.getString(R.string.software_update_check)).performScrollTo()
            val center = compose.onNodeWithTag("settings_notification_center")
            center.performScrollTo()
            val scroller = compose.onNode(hasScrollAction())
            fun range() = scroller.fetchSemanticsNode()
                .config[androidx.compose.ui.semantics.SemanticsProperties.VerticalScrollAxisRange]
            // A tall tablet may show every setting at once. If it can scroll,
            // always exercise a nonzero offset without moving the action off-screen.
            if (range().maxValue() > 0f && range().value() == 0f) {
                scroller.performSemanticsAction(SemanticsActions.ScrollBy) { it(0f, 12f) }
            }
            val before = range().value()
            org.junit.Assert.assertTrue("Scrollable General must exercise a reading position", range().maxValue() == 0f || before > 0f)
            center.performClick()
            back(activity)
            center.assertIsDisplayed()
            org.junit.Assert.assertEquals("Returning from notifications preserves the reading position", before, range().value(), 1f)
        }
    }

}
