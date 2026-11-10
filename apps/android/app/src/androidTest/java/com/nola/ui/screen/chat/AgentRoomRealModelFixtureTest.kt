package com.nola.ui.screen.chat

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import java.io.File
import java.net.URI
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.json.JSONObject
import org.junit.Test
import org.junit.Assume.assumeTrue
import org.junit.runner.RunWith

/** Opt-in preparation of a disposable emulator fixture; never clears application data. */
@RunWith(AndroidJUnit4::class)
class AgentRoomRealModelFixtureTest {
    @Test
    fun removeTemporaryModelCredential() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("clearRoomFixtureCredential") == "true")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val secureStore = SecureTokenStore(context)
        secureStore.deleteLocalModelApiKey()
        check(secureStore.readLocalModelApiKey() == null)
        // Instrumentation may end before SharedPreferences.apply reaches disk.
        check(context.getSharedPreferences("nola_secure_tokens", android.content.Context.MODE_PRIVATE).edit().commit())
    }

    @Test
    fun configureDisposableEmulatorModel() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = File(context.filesDir, "nola-room-real-model-fixture.json")
        assumeTrue("Requires an explicitly staged disposable fixture configuration", file.isFile)
        val config = JSONObject(file.readText())
        check(config.getBoolean("disposableFixture"))
        val endpoint = config.getString("endpoint")
        check(URI(endpoint).host == "192.168.31.18" && URI(endpoint).port == 8081)
        try {
            AndroidApplicationRuntime(context).use { runtime ->
                val profiles = runtime.listModelProfiles().filter {
                    it.provider == "openai-compatible" && it.runtime == "remote" && it.deletedAtMs == null
                }
                // The imported CLI test profile has a CLI-only reasoning preference.
                // Explicitly prepare this disposable fixture for the mobile editor.
                val prepared = profiles.map { profile ->
                    val source = Json.parseToJsonElement(profile.sourceJson) as JsonObject
                    profile.copy(sourceJson = JsonObject(source - "reasoning").toString(),
                        selected = profile.id == profiles.first { it.modelName == config.getString("model") }.id)
                }
                runtime.replaceModelProfiles("openai-compatible", "remote", prepared,
                    profiles.associate { it.id to it.revision }, System.currentTimeMillis())
                runBlocking {
                    MemoryProfileDataStore(context, SecureTokenStore(context), runtime).saveAgentLlmConfig(
                        endpoint, config.getString("apiKey"), config.getString("model"),
                    )
                }
            }
        } finally {
            file.delete()
        }
    }
}
