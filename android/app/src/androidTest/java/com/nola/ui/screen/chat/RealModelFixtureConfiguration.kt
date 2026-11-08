// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.content.Context
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
