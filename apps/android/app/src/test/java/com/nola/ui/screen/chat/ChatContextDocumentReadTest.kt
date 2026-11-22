package com.nola.ui.screen.chat

import java.io.ByteArrayInputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class ChatContextDocumentReadTest {
    @Test
    fun readBoundedBytesCapsStreamAtMaxBytes() {
        val source = ByteArray(100) { it.toByte() }
        val bounded = readBoundedBytes(ByteArrayInputStream(source), maxBytes = 25)
        assertEquals(25, bounded.size)
        assertArrayEquals(source.copyOf(25), bounded)
    }

    @Test
    fun readBoundedBytesReadsShorterStreamCompletely() {
        val source = "hello world".toByteArray(Charsets.UTF_8)
        val bounded = readBoundedBytes(ByteArrayInputStream(source), maxBytes = 1024)
        assertArrayEquals(source, bounded)
    }
}
