package com.nola.ui.screen.chat

import java.io.ByteArrayInputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class ChatAttachmentImportLimitTest {
    @Test
    fun attachmentWithinLimitIsPreservedWithoutTruncation() {
        val source = ByteArray(17) { it.toByte() }

        val imported = readLimitedAttachmentBytes(ByteArrayInputStream(source), maximumBytes = 17)

        assertArrayEquals(source, imported)
    }

    @Test
    fun attachmentAboveLimitIsRejectedInsteadOfSilentlyTruncated() {
        val source = ByteArray(18) { it.toByte() }

        assertThrows(IllegalArgumentException::class.java) {
            readLimitedAttachmentBytes(ByteArrayInputStream(source), maximumBytes = 17)
        }
    }
}
