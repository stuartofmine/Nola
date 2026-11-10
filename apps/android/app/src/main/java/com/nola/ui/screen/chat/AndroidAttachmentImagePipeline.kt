package com.nola.ui.screen.chat

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream

internal const val MAX_MODEL_IMAGE_BYTES = 4 * 1024 * 1024
private const val MAX_MODEL_IMAGE_DIMENSION = 2_048

internal data class PreparedAndroidImage(
    val bytes: ByteArray,
    val mimeType: String,
)

internal fun interface AndroidAttachmentImageCodec {
    fun decodeAndNormalize(bytes: ByteArray): PreparedAndroidImage?
}

internal fun prepareAndroidAttachmentImage(
    name: String,
    declaredMimeType: String,
    bytes: ByteArray,
    codec: AndroidAttachmentImageCodec = PlatformAndroidAttachmentImageCodec,
): PreparedAndroidImage? {
    val extension = name.substringAfterLast('.', "").lowercase()
    val declaredImage = declaredMimeType.startsWith("image/", ignoreCase = true)
    val imageExtension = extension in setOf(
        "png", "jpg", "jpeg", "webp", "gif", "heic", "heif", "tif", "tiff", "bmp", "avif"
    )
    val detectedImage = detectedImageMimeType(bytes) != null
    if (!declaredImage && !imageExtension && !detectedImage) return null

    val prepared = requireNotNull(codec.decodeAndNormalize(bytes)) {
        "Attachment is declared or named as an image, but its content cannot be decoded"
    }
    require(prepared.mimeType.startsWith("image/", ignoreCase = true)) {
        "Image normalization returned a non-image media type"
    }
    require(prepared.bytes.isNotEmpty()) { "Image normalization returned no data" }
    require(prepared.bytes.size <= MAX_MODEL_IMAGE_BYTES) {
        "Normalized image exceeds the ${MAX_MODEL_IMAGE_BYTES / 1024 / 1024} MB model payload limit"
    }
    return prepared
}

internal fun detectedImageMimeType(bytes: ByteArray): String? = when {
    bytes.startsWith(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a) -> "image/png"
    bytes.startsWith(0xff, 0xd8, 0xff) -> "image/jpeg"
    bytes.startsWithAscii("GIF87a") || bytes.startsWithAscii("GIF89a") -> "image/gif"
    bytes.startsWithAscii("BM") -> "image/bmp"
    bytes.size >= 12 && bytes.startsWithAscii("RIFF") && bytes.asciiAt(8, "WEBP") -> "image/webp"
    bytes.startsWith(0x49, 0x49, 0x2a, 0x00) || bytes.startsWith(0x4d, 0x4d, 0x00, 0x2a) -> "image/tiff"
    bytes.size >= 12 && bytes.asciiAt(4, "ftyp") && bytes.asciiAt(8, "avif") -> "image/avif"
    bytes.size >= 12 && bytes.asciiAt(4, "ftyp") && bytes.asciiAt(8, "avis") -> "image/avif"
    bytes.size >= 12 && bytes.asciiAt(4, "ftyp") && bytes.asciiAt(8, "heic") -> "image/heic"
    bytes.size >= 12 && bytes.asciiAt(4, "ftyp") && bytes.asciiAt(8, "heix") -> "image/heic"
    bytes.size >= 12 && bytes.asciiAt(4, "ftyp") && bytes.asciiAt(8, "mif1") -> "image/heif"
    else -> null
}

private object PlatformAndroidAttachmentImageCodec : AndroidAttachmentImageCodec {
    override fun decodeAndNormalize(bytes: ByteArray): PreparedAndroidImage? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null

        var sampleSize = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / sampleSize > MAX_MODEL_IMAGE_DIMENSION) {
            sampleSize *= 2
        }
        val options = BitmapFactory.Options().apply {
            inSampleSize = sampleSize
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: return null
        return try {
            encodeBoundedJpeg(decoded)
        } finally {
            decoded.recycle()
        }
    }

    private fun encodeBoundedJpeg(decoded: Bitmap): PreparedAndroidImage? {
        var current = scaleWithin(decoded, MAX_MODEL_IMAGE_DIMENSION)
        try {
            for ((dimension, quality) in listOf(2_048 to 82, 1_600 to 76, 1_280 to 70, 1_024 to 64)) {
                val resized = scaleWithin(current, dimension)
                if (resized !== current) {
                    if (current !== decoded) current.recycle()
                    current = resized
                }
                val output = ByteArrayOutputStream()
                if (current.compress(Bitmap.CompressFormat.JPEG, quality, output)) {
                    val encoded = output.toByteArray()
                    if (encoded.isNotEmpty() && encoded.size <= MAX_MODEL_IMAGE_BYTES) {
                        return PreparedAndroidImage(encoded, "image/jpeg")
                    }
                }
            }
            return null
        } finally {
            if (current !== decoded) current.recycle()
        }
    }

    private fun scaleWithin(bitmap: Bitmap, maximumDimension: Int): Bitmap {
        val longest = maxOf(bitmap.width, bitmap.height)
        if (longest <= maximumDimension) return bitmap
        val scale = maximumDimension.toFloat() / longest.toFloat()
        return Bitmap.createScaledBitmap(
            bitmap,
            maxOf(1, (bitmap.width * scale).toInt()),
            maxOf(1, (bitmap.height * scale).toInt()),
            true,
        )
    }
}

private fun ByteArray.startsWith(vararg prefix: Int): Boolean =
    size >= prefix.size && prefix.indices.all { this[it].toInt() and 0xff == prefix[it] }

private fun ByteArray.startsWithAscii(prefix: String): Boolean = asciiAt(0, prefix)

private fun ByteArray.asciiAt(offset: Int, value: String): Boolean =
    size >= offset + value.length && value.indices.all { this[offset + it].toInt() == value[it].code }
