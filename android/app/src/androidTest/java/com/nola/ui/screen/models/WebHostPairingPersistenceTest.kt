// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.models

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class WebHostPairingPersistenceTest {
    @Test fun actualNolaServerHandshakeRejectsWrongCredentialAndIdentity() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        requireIsolatedProductTestUser(context)
        org.junit.Assume.assumeTrue(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("realHostPairing") == "1")
        val fixture = org.json.JSONObject(java.io.File(context.filesDir, "host-connection-fixture.json").readText())
        val endpoint = fixture.getString("origin")
        val token = fixture.getString("access_token")
        val expected = fixture.getString("host_id")
        val client = com.nola.di.NetworkModule.provideKtorClient()
        try {
            val api = com.nola.data.remote.api.WebHostApiServiceImpl(client,
                com.nola.data.remote.sse.SseClient(client), com.nola.data.remote.api.WebHostHttpClientFactory(client))
            val settings = api.getAgentSettings(endpoint, token, "")
            assertEquals(expected, verifyWebHostConnection("", expected) { settings })
            assertTrue(runCatching { api.getAgentSettings(endpoint, "wrong-fixture-token", "") }.isFailure)
            val changed = runCatching { verifyWebHostConnection("wrong-host-fixture", null) { settings } }.exceptionOrNull()
            assertEquals(WebHostVerificationFailure.IdentityChanged, (changed as WebHostVerificationException).failure)
        } finally { client.close() }
    }

    @Test fun verifiedPairingPersistsTogetherAndMismatchPreservesCredential() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        requireIsolatedProductTestUser(context)
        val tokens = SecureTokenStore(context)
        val prefs = PreferenceDataStore(context, tokens)
        val oldUrl = prefs.webHostBaseUrl.first()
        val oldToken = prefs.webHostToken.first()
        val oldIdentity = prefs.webHostIdentity.first()
        val oldPin = prefs.webHostCertificateSha256.first()
        try {
            prefs.clearWebHostConfig()
            prefs.saveWebHostConfig("http://first.local:5001", "first-fixture-token", verifiedHostId = "first-host-fixture")
            assertEquals("first-host-fixture", prefs.webHostIdentity.first())
            assertTrue(runCatching { prefs.saveWebHostConfig("http://first.local:5001", "wrong-token", verifiedHostId = "different-host-fixture") }.isFailure)
            assertEquals("first-fixture-token", prefs.webHostToken.first())
            assertEquals("first-host-fixture", prefs.webHostIdentity.first())
            prefs.saveWebHostConfig("http://second.local:5001", "second-fixture-token", verifiedHostId = "second-host-fixture")
            assertEquals("second-host-fixture", prefs.webHostIdentity.first())
            assertEquals("http://second.local:5001", prefs.webHostBaseUrl.first())
            assertEquals("second-fixture-token", prefs.webHostToken.first())
        } finally {
            prefs.clearWebHostConfig()
            if (oldUrl.isNotBlank() && oldToken.isNotBlank()) {
                prefs.saveWebHostConfig(oldUrl, oldToken, oldPin, oldIdentity.takeIf { it.isNotBlank() })
            }
        }
    }
}
