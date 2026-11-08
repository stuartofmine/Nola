package com.arm.aichat.gguf

import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class GgufMetadataTest {
    @Test
    fun acceptsOnlyCurrentGgufVersion() {
        assertEquals(
            GgufMetadata.GgufVersion.VALIDATED_V3,
            GgufMetadata.GgufVersion.fromCode(3),
        )
        assertThrows(IOException::class.java) {
            GgufMetadata.GgufVersion.fromCode(1)
        }
        assertThrows(IOException::class.java) {
            GgufMetadata.GgufVersion.fromCode(2)
        }
    }
}
