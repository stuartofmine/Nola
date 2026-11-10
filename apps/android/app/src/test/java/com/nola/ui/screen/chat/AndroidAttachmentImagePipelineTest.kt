package com.nola.ui.screen.chat

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidAttachmentImagePipelineTest {
    @Test
    fun contentSignatureCanCorrectAnUnhelpfulNameAndMimeType() {
        val png = byteArrayOf(
            0x89.toByte(), 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
        )
        var codecCalled = false
        val normalized = byteArrayOf(1, 2, 3)

        val prepared = prepareAndroidAttachmentImage(
            name = "upload.bin",
            declaredMimeType = "application/octet-stream",
            bytes = png,
            codec = AndroidAttachmentImageCodec {
                codecCalled = true
                PreparedAndroidImage(normalized, "image/jpeg")
            },
        )

        assertTrue(codecCalled)
        assertEquals("image/jpeg", prepared?.mimeType)
        assertArrayEquals(normalized, prepared?.bytes)
    }

    @Test
    fun imageMimeTypeDoesNotPromoteUndecodableBinaryData() {
        val codec = AndroidAttachmentImageCodec { null }

        val error = assertThrows(IllegalArgumentException::class.java) {
            prepareAndroidAttachmentImage("payload.bin", "image/png", byteArrayOf(1, 2, 3), codec)
        }

        assertTrue(error.message.orEmpty().contains("cannot be decoded"))
    }

    @Test
    fun imageExtensionDoesNotOverrideConflictingDocumentContent() {
        val pdf = "%PDF-1.7".toByteArray()
        var codecCalled = false

        assertThrows(IllegalArgumentException::class.java) {
            prepareAndroidAttachmentImage(
                "renamed.png",
                "application/pdf",
                pdf,
                AndroidAttachmentImageCodec {
                    codecCalled = true
                    null
                },
            )
        }

        assertTrue(codecCalled)
        assertNull(detectedImageMimeType(pdf))
    }

    @Test
    fun ordinaryDocumentsDoNotInvokeTheImageDecoder() {
        var codecCalled = false

        val prepared = prepareAndroidAttachmentImage(
            "notes.txt",
            "text/plain",
            "hello".toByteArray(),
            AndroidAttachmentImageCodec {
                codecCalled = true
                PreparedAndroidImage(byteArrayOf(1), "image/jpeg")
            },
        )

        assertNull(prepared)
        assertFalse(codecCalled)
    }

    @Test
    fun normalizedPayloadMustStayWithinTheModelMediaLimit() {
        val oversized = ByteArray(MAX_MODEL_IMAGE_BYTES + 1)

        val error = assertThrows(IllegalArgumentException::class.java) {
            prepareAndroidAttachmentImage(
                "photo.jpg",
                "image/jpeg",
                byteArrayOf(0xff.toByte(), 0xd8.toByte(), 0xff.toByte()),
                AndroidAttachmentImageCodec { PreparedAndroidImage(oversized, "image/jpeg") },
            )
        }

        assertTrue(error.message.orEmpty().contains("model payload limit"))
    }
}
