// SPDX-License-Identifier: Apache-2.0
#include <jni.h>

#include <cstdint>
#include <cstring>
#include <string>

#include "nola_core.h"

namespace {

jbyteArray consume_json(JNIEnv* env, char* value) {
    if (value == nullptr) {
        return nullptr;
    }
    const auto length = static_cast<jsize>(std::strlen(value));
    jbyteArray output = env->NewByteArray(length);
    if (output != nullptr && length > 0) {
        env->SetByteArrayRegion(
            output,
            0,
            length,
            reinterpret_cast<const jbyte*>(value)
        );
    }
    nola_core_free_c_string(value);
    return output;
}

const unsigned char* pin_bytes(JNIEnv* env, jbyteArray input, jbyte** pinned, jsize* length) {
    if (input == nullptr) {
        *pinned = nullptr;
        *length = 0;
        return nullptr;
    }
    *length = env->GetArrayLength(input);
    *pinned = env->GetByteArrayElements(input, nullptr);
    return reinterpret_cast<const unsigned char*>(*pinned);
}

void release_bytes(JNIEnv* env, jbyteArray input, jbyte* pinned) {
    if (input != nullptr && pinned != nullptr) {
        env->ReleaseByteArrayElements(input, pinned, JNI_ABORT);
    }
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_application_abi_version());
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeIntelligenceAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_intelligence_abi_version());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationOpen(
    JNIEnv* env,
    jobject,
    jbyteArray path
) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, path, &pinned, &length);
    NolaCoreApplication* store = nola_core_application_open(bytes, static_cast<size_t>(length));
    release_bytes(env, path, pinned);
    return static_cast<jlong>(reinterpret_cast<std::intptr_t>(store));
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationProtocolVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_application_service_protocol_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationSubmit(
    JNIEnv* env, jobject, jlong handle, jbyteArray envelope
) {
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    if (application == nullptr) return nullptr;
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, envelope, &pinned, &length);
    char* result = nola_core_application_submit(application, bytes, static_cast<size_t>(length));
    release_bytes(env, envelope, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationExecute(
    JNIEnv* env,
    jobject,
    jlong handle,
    jbyteArray command
) {
    auto* store = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    if (store == nullptr) {
        return nullptr;
    }
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, command, &pinned, &length);
    char* result = nola_core_application_execute(store, bytes, static_cast<size_t>(length));
    release_bytes(env, command, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationExpertHostOperation(
    JNIEnv* env, jobject, jlong handle, jbyteArray operation
) {
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    if (application == nullptr) return nullptr;
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, operation, &pinned, &length);
    char* result = nola_core_application_expert_host_operation(application, bytes, static_cast<size_t>(length));
    release_bytes(env, operation, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationAttestEditedResendHostPolicy(
    JNIEnv* env, jobject, jlong handle, jbyteArray target, jbyteArray config
) {
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    if (application == nullptr) return nullptr;
    jbyte* target_pinned = nullptr;
    jbyte* config_pinned = nullptr;
    jsize target_length = 0;
    jsize config_length = 0;
    const unsigned char* target_bytes = pin_bytes(env, target, &target_pinned, &target_length);
    const unsigned char* config_bytes = pin_bytes(env, config, &config_pinned, &config_length);
    char* result = nola_core_application_attest_resend_host_policy(
        application, target_bytes, static_cast<size_t>(target_length),
        config_bytes, static_cast<size_t>(config_length));
    release_bytes(env, config, config_pinned);
    release_bytes(env, target, target_pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT void JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationFree(JNIEnv*, jobject, jlong handle) {
    nola_core_application_free(
        reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle))
    );
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeMemorySelectionAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_memory_selection_abi_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeMemorySelect(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_memory_select_json(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeDataSourceAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_data_source_abi_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeDataSourceResolve(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_data_source_resolve(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeResourceAccessAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_resource_access_abi_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeResourceAccessEvaluate(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_resource_access_evaluate_json(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeToolPolicy(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_tool_policy_resolve(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativeToolPolicyAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_tool_policy_abi_version());
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreNative_nativePromptComposerAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_prompt_composer_abi_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeComposeAgentPrompt(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_compose_agent_prompt_json(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreContextNative_nativeContextPrepare(
    JNIEnv* env,
    jobject,
    jbyteArray input
) {
    jbyte* bytes = nullptr;
    jsize length = 0;
    if (!pin_bytes(env, input, &bytes, &length)) {
        return nullptr;
    }
    char* result = nola_core_context_prepare(
        reinterpret_cast<const uint8_t*>(bytes),
        static_cast<size_t>(length)
    );
    release_bytes(env, input, bytes);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_nola_core_AgentCoreContextNative_nativeContextPrepareAbiVersion(JNIEnv*, jobject) {
    return static_cast<jint>(nola_core_context_engine_abi_version());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeParseAgentModelOutput(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_parse_agent_model_output(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeStitchAgentModelContinuation(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_stitch_agent_model_continuation(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationIntelligenceIngest(JNIEnv* env, jobject, jlong handle, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    char* result = nola_core_application_intelligence_ingest(application, bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationIntelligenceRecall(JNIEnv* env, jobject, jlong handle, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    char* result = nola_core_application_intelligence_recall(application, bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationIntelligenceSnapshot(JNIEnv* env, jobject, jlong handle) {
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    return consume_json(env, nola_core_application_intelligence_snapshot(application));
}

extern "C" JNIEXPORT jboolean JNICALL
Java_com_nola_core_AgentCoreNative_nativeApplicationIntelligenceClear(JNIEnv*, jobject, jlong handle) {
    auto* application = reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle));
    return nola_core_application_intelligence_clear(application) ? JNI_TRUE : JNI_FALSE;
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_nola_core_AgentCoreNative_nativeChildPumpStart(JNIEnv* env, jobject, jlong handle, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const auto* bytes = pin_bytes(env, input, &pinned, &length);
    char* error = nullptr;
    auto* pump = nola_core_child_pump_start(
        reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle)),
        bytes,
        static_cast<size_t>(length),
        &error
    );
    release_bytes(env, input, pinned);
    if (error) {
        if (jclass cls = env->FindClass("java/lang/IllegalStateException")) {
            env->ThrowNew(cls, error);
        }
        nola_core_free_c_string(error);
    }
    return static_cast<jlong>(reinterpret_cast<std::intptr_t>(pump));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeChildPumpPoll(JNIEnv* env, jobject, jlong handle, jint wait_ms) {
    return consume_json(
        env,
        nola_core_child_pump_poll(
            reinterpret_cast<NolaCoreChildPump*>(static_cast<std::intptr_t>(handle)),
            static_cast<uint32_t>(wait_ms)
        )
    );
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeChildPumpRespond(JNIEnv* env, jobject, jlong handle, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const auto* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_child_pump_respond(
        reinterpret_cast<NolaCoreChildPump*>(static_cast<std::intptr_t>(handle)),
        bytes,
        static_cast<size_t>(length)
    );
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT void JNICALL
Java_com_nola_core_AgentCoreNative_nativeChildPumpClose(JNIEnv*, jobject, jlong handle) {
    nola_core_child_pump_close(
        reinterpret_cast<NolaCoreChildPump*>(static_cast<std::intptr_t>(handle))
    );
}

extern "C" JNIEXPORT void JNICALL
Java_com_nola_core_AgentCoreNative_nativeChildPumpFree(JNIEnv*, jobject, jlong handle) {
    nola_core_child_pump_free(
        reinterpret_cast<NolaCoreChildPump*>(static_cast<std::intptr_t>(handle))
    );
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_nola_core_AgentCoreNative_nativeMobileMcpRetain(JNIEnv*, jobject, jlong handle) {
    return static_cast<jlong>(reinterpret_cast<std::intptr_t>(
        nola_core_mobile_mcp_retain(
            reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle))
        )
    ));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeGithubSkillPackage(
    JNIEnv* env, jobject, jbyteArray root, jbyteArray request
) {
    if (!root || !request) return nullptr;
    std::string path(env->GetArrayLength(root), '\0');
    std::string body(env->GetArrayLength(request), '\0');
    env->GetByteArrayRegion(root, 0, path.size(), reinterpret_cast<jbyte*>(path.data()));
    env->GetByteArrayRegion(request, 0, body.size(), reinterpret_cast<jbyte*>(body.data()));
    if (env->ExceptionCheck() || path.find('\0') != std::string::npos || body.find('\0') != std::string::npos)
        return nullptr;
    return consume_json(env, nola_mobile_github_skill_package_json(path.c_str(), body.c_str()));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeMobileMcpHost(JNIEnv* env, jobject, jlong handle, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_mobile_mcp_host(
        reinterpret_cast<NolaCoreApplication*>(static_cast<std::intptr_t>(handle)),
        bytes,
        static_cast<size_t>(length)
    );
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_nola_core_AgentCoreNative_nativeSoftwareUpdate(JNIEnv* env, jobject, jbyteArray input) {
    jbyte* pinned = nullptr;
    jsize length = 0;
    const unsigned char* bytes = pin_bytes(env, input, &pinned, &length);
    char* result = nola_core_software_update_json(bytes, static_cast<size_t>(length));
    release_bytes(env, input, pinned);
    return consume_json(env, result);
}
