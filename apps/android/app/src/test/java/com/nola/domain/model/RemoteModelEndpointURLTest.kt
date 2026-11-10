package com.nola.domain.model

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteModelEndpointURLTest {
    @Test
    fun acceptsHttpAndHttpsHosts() {
        assertTrue(RemoteModelEndpointURL.isValid("https://api.example.com/v1"))
        assertTrue(RemoteModelEndpointURL.isValid(" http://127.0.0.1:11434/v1 "))
    }

    @Test
    fun rejectsUnsupportedOrAmbiguousAddresses() {
        listOf(
            "", "api.example.com/v1", "file:///tmp/model", "ftp://api.example.com",
            "https://user@api.example.com/v1", "https://user:secret@api.example.com/v1",
            "https://api.example.com/v1#fragment", "https:///v1",
        ).forEach { assertFalse(it, RemoteModelEndpointURL.isValid(it)) }
    }
}
