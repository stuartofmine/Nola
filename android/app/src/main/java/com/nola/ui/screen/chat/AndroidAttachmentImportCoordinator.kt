package com.nola.ui.screen.chat

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import com.nola.core.IntelligenceEvent
import com.nola.core.IntelligenceIngestRequest
import com.nola.core.IntelligenceMemoryCandidate
import com.nola.core.PersonalIntelligenceCore
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.zip.ZipInputStream
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val MAX_CHAT_ATTACHMENT_CHARS = 40_000
private const val MAX_CHAT_ATTACHMENT_BYTES = 30 * 1024 * 1024

internal data class AndroidAttachmentImportRequest(
    val uris: List<Uri>,
)

internal data class AndroidAttachmentImportFailure(
    val uri: Uri,
    val displayName: String,
)

internal data class AndroidAttachmentImportResult(
    val attachments: List<AndroidChatAttachment>,
    val failures: List<AndroidAttachmentImportFailure>,
) {
    val failedCount: Int
        get() = failures.size
}

internal fun interface AndroidAttachmentImporter {
    suspend fun import(request: AndroidAttachmentImportRequest): AndroidAttachmentImportResult
}

@kotlinx.serialization.Serializable
internal data class AndroidChatAttachment(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val typeLabel: String,
    val sizeBytes: Int,
    val content: String,
    val extractionLabel: String,
    val contentDigest: String,
    val uri: String,
    val mimeType: String,
    val base64Data: String? = null,
) {
    val sizeLabel: String
        get() = when {
            sizeBytes < 1024 -> "$sizeBytes B"
            sizeBytes < 1024 * 1024 -> "%.1f KB".format(sizeBytes / 1024.0)
            else -> "%.1f MB".format(sizeBytes / (1024.0 * 1024.0))
        }
}

/**
 * Owns Android document-provider IO and attachment decoding for the chat composer.
 *
 * Input is a batch of content URIs. Output preserves URI order and reports each failed
 * source without exposing resolver, codec, or intelligence-ingest details to the UI.
 */
internal class AndroidAttachmentImportCoordinator(
    context: Context,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) : AndroidAttachmentImporter {
    private val applicationContext = context.applicationContext
    private val contentResolver: ContentResolver = applicationContext.contentResolver

    override suspend fun import(request: AndroidAttachmentImportRequest): AndroidAttachmentImportResult =
        withContext(ioDispatcher) {
            val attachments = mutableListOf<AndroidChatAttachment>()
            val failures = mutableListOf<AndroidAttachmentImportFailure>()

            request.uris.forEach { uri ->
                runCatching {
                    importOne(uri).also(::recordImport)
                }
                    .onSuccess { attachment ->
                        attachments += attachment
                    }
                    .onFailure { error ->
                        val displayName = uri.lastPathSegment ?: defaultAttachmentName()
                        failures += AndroidAttachmentImportFailure(uri, displayName)
                        recordFailure(displayName, error)
                    }
            }

            AndroidAttachmentImportResult(attachments, failures)
        }

    private fun importOne(uri: Uri): AndroidChatAttachment {
        runCatching {
            contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        val bytes = contentResolver.openInputStream(uri)?.use { input ->
            readLimitedAttachmentBytes(input)
        } ?: ByteArray(0)
        val name = displayName(uri) ?: uri.lastPathSegment ?: defaultAttachmentName()
        val declaredMimeType = contentResolver.getType(uri) ?: "*/*"
        val parsed = parseAttachment(name, declaredMimeType, bytes)
        val attachmentId = UUID.randomUUID().toString()
        val directory = java.io.File(applicationContext.filesDir, "chat-attachments").also { check(it.isDirectory || it.mkdirs()) }
        val ownedFile = java.io.File(directory, attachmentId)
        ownedFile.writeBytes(bytes)
        val ownedUri = androidx.core.content.FileProvider.getUriForFile(applicationContext,
            "${applicationContext.packageName}.fileprovider", ownedFile)
        return AndroidChatAttachment(
            id = attachmentId,
            name = name,
            typeLabel = parsed.typeLabel,
            sizeBytes = bytes.size,
            content = parsed.content,
            extractionLabel = parsed.extractionLabel,
            contentDigest = attachmentDigest(bytes),
            uri = ownedUri.toString(),
            mimeType = parsed.image?.mimeType ?: declaredMimeType,
            base64Data = parsed.image?.bytes?.let { normalizedBytes ->
                Base64.encodeToString(normalizedBytes, Base64.NO_WRAP)
            },
        )
    }

    private fun displayName(uri: Uri): String? =
        contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
        }

    private fun defaultAttachmentName(): String =
        applicationContext.getString(com.nola.R.string.chat_attachment_default_name)

    private fun parseAttachment(
        name: String,
        declaredMimeType: String,
        bytes: ByteArray,
    ): ParsedAndroidAttachment {
        val image = prepareAndroidAttachmentImage(name, declaredMimeType, bytes)
        if (image != null) {
            return ParsedAndroidAttachment("IMAGE", "Image", "", image)
        }

        val extension = name.substringAfterLast('.', "").lowercase()
        return when (extension) {
            "pdf" -> {
                com.tom_roush.pdfbox.android.PDFBoxResourceLoader.init(applicationContext)
                val text = com.tom_roush.pdfbox.pdmodel.PDDocument.load(bytes).use { document ->
                    com.tom_roush.pdfbox.text.PDFTextStripper().getText(document)
                }.trim().take(MAX_CHAT_ATTACHMENT_CHARS)
                require(text.isNotEmpty()) { "PDF does not contain extractable text" }
                ParsedAndroidAttachment("PDF", "PDF text", text)
            }
            "docx" -> {
                val text = extractDocxText(bytes).trim().take(MAX_CHAT_ATTACHMENT_CHARS)
                require(text.isNotEmpty()) { "DOCX does not contain extractable text" }
                ParsedAndroidAttachment("DOCX", "DOCX text", text)
            }
            "txt", "md", "markdown", "csv", "tsv", "json", "xml", "yaml", "yml", "log",
            "kt", "kts", "java", "swift", "rs", "py", "js", "ts", "tsx", "jsx", "html", "css" -> {
                val text = bytes.toString(Charsets.UTF_8).trim().take(MAX_CHAT_ATTACHMENT_CHARS)
                require(text.isNotEmpty()) { "Text file is empty" }
                ParsedAndroidAttachment(extension.uppercase(), "Local text", text)
            }
            else -> error("Unsupported attachment type: .$extension")
        }
    }

    private fun recordImport(attachment: AndroidChatAttachment) {
        val eventId = UUID.randomUUID().toString()
        val tags = listOf(
            "attachment",
            "document",
            "文件",
            attachment.typeLabel.lowercase(),
            attachment.name.lowercase(),
        )
        val excerpt = attachment.content.trim().take(1_600)
        PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(
                events = listOf(
                    IntelligenceEvent(
                        id = eventId,
                        source = "attachment",
                        kind = "document.imported",
                        subject = attachment.contentDigest,
                        summary = "Imported ${attachment.name} (${attachment.typeLabel}, " +
                            "${attachment.extractionLabel}, ${attachment.sizeBytes} bytes)",
                        importance = 0.7,
                        confidence = 1.0,
                        tags = tags,
                        dedupe_key = "attachment.import.${attachment.contentDigest}",
                    ),
                ),
                memory_candidates = if (excerpt.isEmpty()) {
                    emptyList()
                } else {
                    listOf(
                        IntelligenceMemoryCandidate(
                            semantic_key = "document.${attachment.contentDigest}",
                            statement = "File ${attachment.name} (${attachment.typeLabel}, " +
                                "${attachment.extractionLabel}) contains: $excerpt",
                            confidence = 0.92,
                            source_event_ids = listOf(eventId),
                            tags = tags,
                        ),
                    )
                },
            ),
        )
    }

    private fun recordFailure(name: String, error: Throwable) {
        val reason = error.message ?: error::class.java.simpleName
        PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(
                events = listOf(
                    IntelligenceEvent(
                        source = "attachment",
                        kind = "document.import_failed",
                        subject = name,
                        summary = "Failed to import $name: $reason",
                        importance = 0.3,
                        confidence = 1.0,
                        tags = listOf("attachment", "failure", "文件", name.lowercase()),
                        dedupe_key = "attachment.failure.$name.$reason",
                    ),
                ),
            ),
        )
    }
}

private data class ParsedAndroidAttachment(
    val typeLabel: String,
    val extractionLabel: String,
    val content: String,
    val image: PreparedAndroidImage? = null,
)

private fun extractDocxText(bytes: ByteArray): String {
    ZipInputStream(ByteArrayInputStream(bytes)).use { archive ->
        while (true) {
            val entry = archive.nextEntry ?: break
            if (entry.name == "word/document.xml") {
                val xml = archive.readBytes().toString(Charsets.UTF_8)
                return xml
                    .replace(Regex("</w:p>"), "\n")
                    .replace(Regex("</w:tab>"), "\t")
                    .replace(Regex("<[^>]+>"), "")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&apos;", "'")
            }
        }
    }
    return ""
}

private fun attachmentDigest(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256")
        .digest(bytes)
        .joinToString("") { byte -> "%02x".format(byte) }

internal fun readLimitedAttachmentBytes(
    input: InputStream,
    maximumBytes: Int = MAX_CHAT_ATTACHMENT_BYTES,
): ByteArray {
    val output = ByteArrayOutputStream()
    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
    var remaining = maximumBytes + 1
    while (remaining > 0) {
        val read = input.read(buffer, 0, minOf(buffer.size, remaining))
        if (read <= 0) break
        output.write(buffer, 0, read)
        remaining -= read
    }
    val bytes = output.toByteArray()
    require(bytes.size <= maximumBytes) {
        "Attachment exceeds the ${maximumBytes / 1024 / 1024} MB limit"
    }
    return bytes
}
