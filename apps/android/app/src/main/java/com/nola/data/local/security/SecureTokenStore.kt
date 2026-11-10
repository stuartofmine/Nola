package com.nola.data.local.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import dagger.hilt.android.qualifiers.ApplicationContext
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class SecureTokenStore @Inject constructor(
    @ApplicationContext context: Context
) {
    private val preferences = context.getSharedPreferences("nola_secure_tokens", Context.MODE_PRIVATE)

    fun saveWebHostToken(token: String): Boolean = saveSecret(WEB_HOST_TOKEN, token)

    fun readWebHostToken(): String? = readSecret(WEB_HOST_TOKEN)

    fun deleteWebHostToken() = deleteSecret(WEB_HOST_TOKEN)

    fun saveBackendAccessToken(token: String): Boolean = saveSecret(BACKEND_ACCESS_TOKEN, token)

    fun readBackendAccessToken(): String? = readSecret(BACKEND_ACCESS_TOKEN)

    fun deleteBackendAccessToken() = deleteSecret(BACKEND_ACCESS_TOKEN)

    fun saveModelAccountSecret(reference: String, value: String): Boolean {
        require(reference.startsWith("android-model:"))
        return saveSecret(reference,value)
    }
    fun readModelAccountSecret(reference: String): String? {
        require(reference.startsWith("android-model:")) { "This account credential is not available on this device" }
        return readSecret(reference)
    }

    fun saveLocalModelApiKey(apiKey: String): Boolean = saveSecret(LOCAL_MODEL_API_KEY, apiKey)

    fun readLocalModelApiKey(): String? = readSecret(LOCAL_MODEL_API_KEY)

    fun deleteLocalModelApiKey() = deleteSecret(LOCAL_MODEL_API_KEY)

    fun clearAll() {
        preferences.edit().clear().commit()
        runCatching {
            val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            if (store.containsAlias(KEY_ALIAS)) store.deleteEntry(KEY_ALIAS)
        }
    }

    private fun saveSecret(name: String, value: String): Boolean = runCatching {
        if (value.isBlank()) {
            deleteSecret(name)
            return true
        }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val ciphertext = cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        val encoded = Base64.getEncoder().encodeToString(cipher.iv + ciphertext)
        preferences.edit().putString(name, encoded).commit()
    }.getOrDefault(false)

    private fun readSecret(name: String): String? = runCatching {
        val encoded = preferences.getString(name, null) ?: return null
        val sealed = Base64.getDecoder().decode(encoded)
        require(sealed.size > NONCE_BYTES + 16)
        val nonce = sealed.copyOfRange(0, NONCE_BYTES)
        val ciphertext = sealed.copyOfRange(NONCE_BYTES, sealed.size)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, nonce))
        String(cipher.doFinal(ciphertext), Charsets.UTF_8)
    }.getOrNull()?.takeIf { it.isNotBlank() }

    private fun deleteSecret(name: String) {
        preferences.edit().remove(name).apply()
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return generator.generateKey()
    }

    private companion object {
        const val KEY_ALIAS = "nola.web_host.token.v1"
        const val WEB_HOST_TOKEN = "web_host_token"
        const val BACKEND_ACCESS_TOKEN = "backend_access_token"
        const val LOCAL_MODEL_API_KEY = "local_model_api_key"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val NONCE_BYTES = 12
    }
}
