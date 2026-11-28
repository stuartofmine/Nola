// SPDX-License-Identifier: Apache-2.0
package com.nola.llm.provider

import kotlinx.serialization.json.JsonObject

/** Production and desktop JVM tests call the same Core ABI and JNI adapter. */
internal object ModelRequestPrivacy {
    init {
        val testLibrary = System.getProperty("nola.test.modelPrivacyLibrary")
        if (testLibrary == null) System.loadLibrary("nola_core_jni") else System.load(testLibrary)
    }

    private external fun redactJson(body: ByteArray, credential: ByteArray, endpoint: String): ByteArray?

    fun redact(body: JsonObject, key: String, endpoint: String): ByteArray =
        checkNotNull(redactJson(body.toString().encodeToByteArray(), key.encodeToByteArray(), endpoint)) {
            "Model request privacy filter unavailable."
        }
}
