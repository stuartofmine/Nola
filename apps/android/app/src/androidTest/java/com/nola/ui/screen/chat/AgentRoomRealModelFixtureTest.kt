package com.nola.ui.screen.chat

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.security.SecureTokenStore
import java.io.File
import kotlinx.coroutines.runBlocking
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
        requireIsolatedProductTestUser(context)
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
        try {
            AndroidApplicationRuntime(context).use { runtime ->
                runBlocking { applyRealModelFixture(context, runtime, config) }
            }
        } finally {
            file.delete()
        }
    }
}
