// SPDX-License-Identifier: Apache-2.0
#include <jni.h>
#include <cstring>
#include <cstdint>
#include "nola_core.h"

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_llm_provider_ModelRequestPrivacy_redactJson(JNIEnv* env, jobject, jbyteArray input, jbyteArray credential, jstring endpoint) {
    const auto length = env->GetArrayLength(input);
    const auto keyLength = env->GetArrayLength(credential);
    jbyte* bytes = env->GetByteArrayElements(input, nullptr);
    if (!bytes) return nullptr;
    jbyte* key = keyLength ? env->GetByteArrayElements(credential, nullptr) : nullptr;
    if (keyLength && !key) {
        env->ReleaseByteArrayElements(input, bytes, JNI_ABORT);
        return nullptr;
    }
    const char* address = env->GetStringUTFChars(endpoint, nullptr);
    char* result = address ? nola_core_redact_model_request_for_endpoint(
        reinterpret_cast<const unsigned char*>(bytes), length,
        reinterpret_cast<const unsigned char*>(key), keyLength, address) : nullptr;
    if (address) env->ReleaseStringUTFChars(endpoint, address);
    if (key) env->ReleaseByteArrayElements(credential, key, JNI_ABORT);
    env->ReleaseByteArrayElements(input, bytes, JNI_ABORT);
    if (!result) return nullptr;
    const size_t size = std::strlen(result);
    jbyteArray output = size <= INT32_MAX ? env->NewByteArray(static_cast<jsize>(size)) : nullptr;
    if (output) env->SetByteArrayRegion(output, 0, static_cast<jsize>(size), reinterpret_cast<const jbyte*>(result));
    nola_model_privacy_free(result);
    return output;
}
