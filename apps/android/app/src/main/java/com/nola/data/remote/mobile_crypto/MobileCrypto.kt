package com.nola.data.remote.mobile_crypto

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

@Serializable
data class EncryptedEnvelope(
    @SerialName("type") val type: String? = null,
    val v: Int = 1,
    val alg: String = "AES-256-GCM",
    val nonce: String,
    val ciphertext: String,
    val tag: String
)

object MobileCrypto {
    private const val AAD = "nola-mobile-bridge-v1"
    private const val ALG = "AES-256-GCM"
    private const val VERSION = 1
    private const val GCM_TAG_LENGTH = 128
    private val json = Json {
        ignoreUnknownKeys = true
        prettyPrint = false
    }

    fun encryptJson(payload: Map<String, Any?>, token: String): EncryptedEnvelope {
        return encryptJsonObject(payload.toJsonObject(), token)
    }

    fun payloadJsonObject(payload: Map<String, Any?>): JsonObject = payload.toJsonObject()

    fun encryptJsonObject(payload: JsonObject, token: String): EncryptedEnvelope {
        require(token.isNotBlank()) { "missing access token" }
        val key = MessageDigest.getInstance("SHA-256").digest(token.toByteArray(Charsets.UTF_8))
        val plaintext = payload.toString().toByteArray(Charsets.UTF_8)

        val nonce = ByteArray(12).also { SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        val spec = GCMParameterSpec(GCM_TAG_LENGTH, nonce)
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), spec)
        cipher.updateAAD(AAD.toByteArray(Charsets.UTF_8))
        val sealed = cipher.doFinal(plaintext)

        val cipherLen = sealed.size - 16
        val ciphertext = sealed.copyOfRange(0, cipherLen)
        val tag = sealed.copyOfRange(cipherLen, sealed.size)

        return EncryptedEnvelope(
            v = VERSION,
            alg = ALG,
            nonce = base64UrlEncode(nonce),
            ciphertext = base64UrlEncode(ciphertext),
            tag = base64UrlEncode(tag)
        )
    }

    fun decryptJsonObject(envelope: EncryptedEnvelope, token: String): JsonObject {
        require(token.isNotBlank()) { "missing access token" }
        require(envelope.v == VERSION && envelope.alg == ALG) { "unsupported encrypted payload" }
        val key = MessageDigest.getInstance("SHA-256").digest(token.toByteArray(Charsets.UTF_8))
        val nonce = base64UrlDecode(envelope.nonce)
        val ciphertext = base64UrlDecode(envelope.ciphertext)
        val tag = base64UrlDecode(envelope.tag)
        require(nonce.size == 12 && tag.size == 16) { "invalid encrypted payload" }

        val sealed = ciphertext + tag
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        val spec = GCMParameterSpec(GCM_TAG_LENGTH, nonce)
        cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), spec)
        cipher.updateAAD(AAD.toByteArray(Charsets.UTF_8))
        val plaintext = cipher.doFinal(sealed)

        return json.parseToJsonElement(String(plaintext, Charsets.UTF_8)).jsonObject
    }

    fun decryptJson(envelope: EncryptedEnvelope, token: String): Map<String, Any?> {
        return decryptJsonObject(envelope, token).toPlainMap()
    }

    private fun base64UrlEncode(data: ByteArray): String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(data)

    private fun base64UrlDecode(data: String): ByteArray {
        val padded = data + "=".repeat((4 - data.length % 4) % 4)
        return Base64.getUrlDecoder().decode(padded)
    }

    private fun Map<String, Any?>.toJsonObject(): JsonObject = buildJsonObject {
        forEach { (key, value) -> put(key, value.toJsonElement()) }
    }

    private fun Any?.toJsonElement(): JsonElement = when (this) {
        null -> JsonNull
        is JsonElement -> this
        is String -> JsonPrimitive(this)
        is Boolean -> JsonPrimitive(this)
        is Int -> JsonPrimitive(this)
        is Long -> JsonPrimitive(this)
        is Float -> JsonPrimitive(this)
        is Double -> JsonPrimitive(this)
        is Number -> JsonPrimitive(this)
        is Map<*, *> -> buildJsonObject {
            this@toJsonElement.forEach { (key, value) ->
                if (key != null) put(key.toString(), value.toJsonElement())
            }
        }
        is Iterable<*> -> buildJsonArray {
            this@toJsonElement.forEach { add(it.toJsonElement()) }
        }
        is Array<*> -> buildJsonArray {
            this@toJsonElement.forEach { add(it.toJsonElement()) }
        }
        else -> JsonPrimitive(toString())
    }

    private fun JsonObject.toPlainMap(): Map<String, Any?> = mapValues { it.value.toPlainValue() }

    private fun JsonElement.toPlainValue(): Any? = when (this) {
        JsonNull -> null
        is JsonObject -> toPlainMap()
        is JsonArray -> map { it.toPlainValue() }
        is JsonPrimitive -> {
            when {
                isString -> content
                booleanOrNull != null -> booleanOrNull
                longOrNull != null -> longOrNull
                doubleOrNull != null -> doubleOrNull
                else -> content
            }
        }
    }
}
