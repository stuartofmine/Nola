// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.test.platform.app.InstrumentationRegistry
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.CanonicalModelProfile
import com.nola.domain.model.ModelParameters
import dagger.hilt.android.EntryPointAccessors
import java.util.UUID
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** Captures the permissions and model binding through the same adapter as the Goal sheet. */
class AndroidGoalStartAdmissionTest {
    @Test fun firstGoalObservesDeviceCatalogAndModelWithoutReplayingTranscript() = runBlocking<Unit> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        requireIsolatedProductTestUser(context)
        val deps = EntryPointAccessors.fromApplication(context.applicationContext, AndroidExpertNativeTestDependencies::class.java)
        val store = deps.runtime()
        val owner = UUID.randomUUID().toString()
        val profile = "goal-capture-$owner"
        val source = buildJsonObject {
            put("baseURL", "http://127.0.0.1:1/v1")
            put("wireProtocol", "responses")
            put("credentialRequired", false)
            put("capabilities", buildJsonArray { add("chat") })
            put("executionCapabilities", buildJsonObject {
                put("toolCalls", true)
                put("imageInput", false)
                put("reasoning", false)
                put("samplingParameters", false)
            })
        }
        store.upsertModelProfile(CanonicalModelProfile(profile, "Goal capture", "openai-compatible", profile,
            "remote", source.toString(), """{"maxContextLength":32768,"maxOutputTokens":1024}""",
            false, true, 0, 1, 1))
        store.createConversation(owner, "Goal capture", 1, 1, buildJsonObject {
            put("modelSelectionID", "configured:$profile")
        }.toString())
        try {
            val model = deps.modelRepository().listConfiguredModels().single { it.profileId == profile }
            val parameters = ModelParameters(maxContextTokens = 32768, maxCompletionTokens = 1024)
            suspend fun capture(start: Boolean) = deps.turnService().goalModelAdmission(owner, model, parameters,
                false, parameters.responseStyle, start, false, AndroidAgentPermissionMode.Cautious)
            val start = capture(true)
            val config = checkNotNull(start.startConfiguration)
            assertEquals(owner, config.getValue("session_id").jsonPrimitive.content)
            assertTrue(config.getValue("initial_items").jsonArray.isEmpty())
            val catalog = config.getValue("host_catalog").jsonObject.getValue("capabilities").jsonObject
            assertTrue("A prior terminal checkpoint drops even the stop handler", "host.cancel" in catalog)
            assertTrue(config.getValue("tools").jsonArray.isNotEmpty())
            val invocation = config.getValue("turn_identity").jsonObject.getValue("host_invocation").jsonObject
            start.selection.forEach { (field, value) -> assertEquals(value, invocation[field]) }
            val resume = capture(false)
            assertEquals(start.selection, resume.selection)
            assertNull("Resume must keep the original frozen Goal catalog", resume.startConfiguration)
        } finally {
            runCatching { store.deleteConversation(owner) }
            runCatching { store.deleteModelProfile(profile, System.currentTimeMillis()) }
        }
    }
}
