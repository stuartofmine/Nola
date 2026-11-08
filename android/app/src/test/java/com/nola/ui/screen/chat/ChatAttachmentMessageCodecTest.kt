package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatAttachmentMessageCodecTest {
    @Test
    fun metadataRoundTripPreservesDisplayTextAndAttachmentFields() {
        val attachment = AndroidChatAttachment(
            name = "财报 \"最终版\".pdf",
            typeLabel = "application/pdf",
            sizeBytes = 4096,
            content = "report text",
            extractionLabel = "PDF text",
            contentDigest = "digest",
            uri = "content://documents/report%20final",
            mimeType = "application/pdf",
        )

        val metadata = attachmentMessageMeta("分析这份报告", listOf(attachment))
        val decoded = decodeMessageAttachments(metadata)

        assertEquals("分析这份报告", metadata?.get("nola.displayMessage"))
        assertEquals(1, decoded.size)
        assertEquals(attachment.name, decoded.single().name)
        assertEquals(attachment.uri, decoded.single().uri)
        assertEquals(attachment.typeLabel, decoded.single().type)
        assertEquals(attachment.sizeBytes, decoded.single().sizeBytes)
    }

    @Test
    fun metadataRoundTripEscapesNewlinesAndQuotes() {
        val attachment = AndroidChatAttachment(
            name = "line one\n\"line two\".txt",
            typeLabel = "text/plain",
            sizeBytes = 12,
            content = "text",
            extractionLabel = "Local text",
            contentDigest = "digest",
            uri = "content://documents/text",
            mimeType = "text/plain",
        )

        val decoded = decodeMessageAttachments(attachmentMessageMeta("", listOf(attachment)))

        assertEquals(attachment.name, decoded.single().name)
    }

    @Test
    fun malformedMetadataReturnsNoAttachments() {
        assertTrue(decodeMessageAttachments(mapOf("nola.attachments" to "{not-json")).isEmpty())
    }
}
