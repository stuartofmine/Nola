// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.content.Context
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.ui.screen.models.MODEL_ROW_TEST_TAG_PREFIX
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import kotlinx.serialization.json.*
import org.json.JSONObject

/** Explicit disposable-AVD setup; never deletes another configured endpoint.
 * A matching live current profile is a no-op, including its revision. Only the
 * test-owned credential is restored to Keystore before the UI selects it.
 */
internal suspend fun applyRealModelFixture(context: Context, runtime: AndroidApplicationRuntime, fixture: JSONObject) {
    requireIsolatedProductTestUser(context)
    val endpoint = fixture.getString("endpoint").trim()
    val model = fixture.getString("model").trim()
    val protocol = fixture.getString("protocol")
    val tokens = SecureTokenStore(context)
    val memory = MemoryProfileDataStore(context, tokens, runtime)
    val before = runtime.listModelProfiles()
    val alreadyConfigured = before.any { profile ->
        if (profile.provider != "openai-compatible" || profile.runtime != "remote" ||
            profile.deletedAtMs != null || !profile.enabled || profile.modelName != model) false
        else Json.parseToJsonElement(profile.sourceJson).jsonObject.let {
            it["baseURL"]?.jsonPrimitive?.content == endpoint && it["wireProtocol"]?.jsonPrimitive?.content == protocol
        }
    }
    if (alreadyConfigured) {
        check(tokens.saveLocalModelApiKey(fixture.getString("apiKey").trim())) { "Unable to restore the isolated fixture credential" }
        check(runtime.listModelProfiles() == before) { "A matching fixture setup changed the model catalog" }
    } else memory.saveAgentLlmConfig(endpoint, fixture.getString("apiKey"), model, protocol)
}

/** Confirm the fixture model through the same capability editor a user opens. */
internal fun ComposeTestRule.confirmFixtureCapabilities(modelId: String) {
    val context = InstrumentationRegistry.getInstrumentation().targetContext
    val previous = mainClock.autoAdvance
    mainClock.autoAdvance = true
    try {
        onNode(hasContentDescription(context.getString(R.string.models_execution_capabilities)) and
            hasAnyAncestor(hasTestTag(MODEL_ROW_TEST_TAG_PREFIX + modelId))).performScrollTo().performClick()
        val save = onNodeWithText(context.getString(R.string.common_save))
        waitUntil(15_000) {
            onAllNodesWithText(context.getString(R.string.models_execution_capabilities_hint)).fetchSemanticsNodes().isNotEmpty()
        }
        if (save.fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled)) {
            onNodeWithText(context.getString(R.string.common_cancel)).performScrollTo().performClick()
        } else {
            save.performScrollTo().performClick()
        }
        waitUntil(15_000) {
            onAllNodesWithText(context.getString(R.string.models_execution_capabilities_hint)).fetchSemanticsNodes().isEmpty()
        }
    } finally {
        mainClock.autoAdvance = previous
    }
}
