package com.nola.ui.screen.models

import com.nola.data.remote.dto.AgentSettingsResponse
import com.nola.data.remote.dto.HOST_PERMISSION_PROTOCOL_VERSION
import com.nola.data.remote.dto.HostIdentityDto
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class WebHostConnectionWorkflowTest {
    @Test
    fun verifiedIdentityIsReturnedOnlyAfterSettingsFetch() = runTest {
        var fetched = false
        val identity = verifyWebHostConnection(expectedHostIdentity = "") {
            fetched = true
            AgentSettingsResponse(
                hostIdentity = HostIdentityDto("host-studio-fixture", HOST_PERMISSION_PROTOCOL_VERSION)
            )
        }

        assertEquals(true, fetched)
        assertEquals("host-studio-fixture", identity)
    }

    @Test
    fun changedIdentityIsRejectedBeforePersistence() = runTest {
        val error = runCatching {
            verifyWebHostConnection(expectedHostIdentity = "host-old-fixture-01") {
                AgentSettingsResponse(
                    hostIdentity = HostIdentityDto("host-new-fixture-02", HOST_PERMISSION_PROTOCOL_VERSION)
                )
            }
        }.exceptionOrNull()

        assertTrue(error is WebHostVerificationException)
        assertEquals(WebHostVerificationFailure.IdentityChanged, (error as WebHostVerificationException).failure)
    }

    @Test
    fun missingOrUnsupportedIdentityIsRejected() = runTest {
        val error = runCatching {
            verifyWebHostConnection(expectedHostIdentity = "") {
                AgentSettingsResponse(
                    hostIdentity = HostIdentityDto("host-studio-fixture", HOST_PERMISSION_PROTOCOL_VERSION - 1)
                )
            }
        }.exceptionOrNull()

        assertTrue(error is WebHostVerificationException)
        assertEquals(WebHostVerificationFailure.UnsupportedProtocol, (error as WebHostVerificationException).failure)
    }

    @Test fun pastedSetupRejectsMalformedOrUnsupportedPayloads() {
        val valid = """{"type":"nola.server.connection","protocol_version":1,"origin":"http://192.168.1.8:5001","host_id":"host-studio-fixture","access_token":"fixture-token"}"""
        assertEquals("host-studio-fixture", WebHostConnectionSetup.decode(valid)?.hostId)
        for (invalid in listOf(valid.replace("\"protocol_version\":1", "\"protocol_version\":true"),
            valid.replace("\"protocol_version\":1", "\"protocol_version\":2"),
            valid.replace("192.168.1.8:5001", "user:pass@192.168.1.8:5001"),
            valid.replace("192.168.1.8:5001", "192.168.1.8:5001?token=secret"),
            valid.replace("192.168.1.8:5001", "8.8.8.8:5001"), "{}")) {
            assertEquals(null, WebHostConnectionSetup.decode(invalid))
        }
    }

    @Test fun originComparisonSeparatesChangingHostFromChangingUrlSpelling() {
        assertTrue(sameWebHostOrigin("https://Studio.local/", "https://studio.local:443/api/mobile"))
        assertEquals(false, sameWebHostOrigin("http://one.local:5001", "http://two.local:5001"))
        assertEquals(false, sameWebHostOrigin("http://one.local:5001", "http://one.local:5002"))
        assertEquals(false, sameWebHostOrigin("http://one.local", "https://one.local"))
    }

    @Test fun pastedIdentityMismatchAndMalformedIdentityFailBeforeSaving() = runTest {
        val mismatch = runCatching { verifyWebHostConnection("", "host-other-fixture") {
            AgentSettingsResponse(hostIdentity = HostIdentityDto("host-studio-fixture", HOST_PERMISSION_PROTOCOL_VERSION))
        } }.exceptionOrNull()
        assertEquals(WebHostVerificationFailure.IdentityChanged, (mismatch as WebHostVerificationException).failure)
        val malformed = runCatching { verifyWebHostConnection("") {
            AgentSettingsResponse(hostIdentity = HostIdentityDto("short", HOST_PERMISSION_PROTOCOL_VERSION))
        } }.exceptionOrNull()
        assertEquals(WebHostVerificationFailure.UnsupportedProtocol, (malformed as WebHostVerificationException).failure)
    }
    @Test fun savedCredentialsAreNeverForwardedToAnotherDestination() {
        val saved = "saved-fixture-token"
        assertEquals(saved, webHostConnectionToken("https://Studio.local", saved, "https://studio.local:443/api/mobile", ""))
        for (destination in listOf("https://other.local", "http://studio.local", "https://studio.local:444", "https://studio.local/other")) {
            assertEquals("", webHostConnectionToken("https://studio.local", saved, destination, ""))
            assertEquals("new-fixture-token", webHostConnectionToken("https://studio.local", saved, destination, " new-fixture-token "))
        }
    }
}
