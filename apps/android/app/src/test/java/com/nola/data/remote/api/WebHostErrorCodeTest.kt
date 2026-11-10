package com.nola.data.remote.api

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Test

class WebHostErrorCodeTest {
    @Test fun onlyBoundedProtocolCodesReachStatusMessages() {
        assertEquals("owner_mismatch", boundedWebHostErrorCode(buildJsonObject { put("code", "owner_mismatch") }))
        for (value in listOf("secret token", "unexpected\nbody", "x".repeat(97), "")) {
            assertNull(boundedWebHostErrorCode(buildJsonObject { put("code", value) }))
        }
        assertNull(boundedWebHostErrorCode(buildJsonObject { put("error", "arbitrary host text") }))
    }
}
