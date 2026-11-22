// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import androidx.activity.ComponentActivity
import androidx.compose.material3.Text
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.di.NetworkModule
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import java.io.File
import java.net.URI

/** The opt-in harness provides an isolated Host behind a bootstrap-only gateway and a real embedded Android AAR. */
class TailcatLiveTransportTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun nativeTransportReadsHostConversationAndReleasesPairing() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue(InstrumentationRegistry.getArguments().getString("liveTailcat") == "1")
        val context = instrumentation.targetContext
        requireIsolatedProductTestUser(context)
        val fixture = JSONObject(File(context.filesDir, "tailcat-live.json").readText())
        val origin = fixture.getString("origin")
        require(URI(origin).host in setOf("10.0.2.2", "127.0.0.1"))
        val token = fixture.getString("token")
        val host = fixture.getString("host")
        val conversation = fixture.getString("conversation")
        val prefs = PreferenceDataStore(context, SecureTokenStore(context))
        compose.setContent { Text("Tailcat transport test") }
        compose.waitUntil(10_000) {
            var started = false
            instrumentation.runOnMainSync { started = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
            started
        }
        runBlocking {
            check(prefs.webHostBaseUrl.first().isBlank())
            val http = NetworkModule.provideKtorClient()
            val transport = NolaTailcatTransport(http, prefs, context)
            val factory = WebHostHttpClientFactory(http, transport)
            val api = WebHostApiServiceImpl(http, SseClient(http), factory)
            var readConversation = false
            try {
                check(transport.available) { "Embedded Tailcat Android JNI binding is required for the live test" }
                prefs.saveWebHostConfig(origin, token, verifiedHostId = host)
                val settings = withTimeout(60_000) { api.getAgentSettings(origin, token) }
                assertEquals(host, settings.hostIdentity?.id)
                val projection = withTimeout(30_000) {
                    api.streamAgentTaskEvents(origin, token, conversation, 0).first {
                        check(it !is SseEvent.Error) { "Remote task stream reported an error" }
                        it is SseEvent.TaskProjection
                    } as SseEvent.TaskProjection
                }
                assertEquals(conversation, projection.conversationId)
                readConversation = true
            } finally {
                prefs.clearWebHostConfig()
                assertTrue(prefs.webHostToken.first().isEmpty())
                // Acknowledging a stop request and actually terminating its helper are distinct.
                val receipt = File(context.filesDir, "tailcat-lease-closed")
                if (readConversation) compose.waitUntil(35_000) { receipt.exists() }
                factory.clientFor(origin, "").close()
                factory.controlClientFor(origin, "").close()
                http.close()
            }
        }
    }
}
