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
                hostIdentity = HostIdentityDto("host-studio", HOST_PERMISSION_PROTOCOL_VERSION)
            )
        }

        assertEquals(true, fetched)
        assertEquals("host-studio", identity)
    }

    @Test
    fun changedIdentityIsRejectedBeforePersistence() = runTest {
        val error = runCatching {
            verifyWebHostConnection(expectedHostIdentity = "host-old") {
                AgentSettingsResponse(
                    hostIdentity = HostIdentityDto("host-new", HOST_PERMISSION_PROTOCOL_VERSION)
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
                    hostIdentity = HostIdentityDto("host-studio", HOST_PERMISSION_PROTOCOL_VERSION - 1)
                )
            }
        }.exceptionOrNull()

        assertTrue(error is WebHostVerificationException)
        assertEquals(WebHostVerificationFailure.UnsupportedProtocol, (error as WebHostVerificationException).failure)
    }
}
