cmake_minimum_required(VERSION 3.22.1)
project(nola_core_jni LANGUAGES C CXX)

set(NOLA_RUST_ARCHIVE "${CMAKE_CURRENT_SOURCE_DIR}/rust/${ANDROID_ABI}/libnola_core.a")
if(NOT EXISTS "${NOLA_RUST_ARCHIVE}")
    message(FATAL_ERROR "Missing Rust archive: ${NOLA_RUST_ARCHIVE}")
endif()

add_library(nola_core STATIC IMPORTED GLOBAL)
set_target_properties(nola_core PROPERTIES IMPORTED_LOCATION "${NOLA_RUST_ARCHIVE}")

add_library(nola_core_jni SHARED nola_core_jni.cpp)
target_include_directories(
    nola_core_jni
    PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../../core/swift_shims"
)

find_library(android_log log)
find_library(android_lib android)
find_library(android_dl dl)
find_library(android_m m)

target_link_libraries(
    nola_core_jni
    PRIVATE
    nola_core
    ${android_log}
    ${android_lib}
    ${android_dl}
    ${android_m}
)

# Pinned official llama.cpp Android runtime. The model is never loaded during
# library initialization; Kotlin explicitly controls load/unload lifecycle.
set(LLAMA_SRC "${CMAKE_CURRENT_SOURCE_DIR}/third_party/llama.cpp")
add_subdirectory("${LLAMA_SRC}" "${CMAKE_CURRENT_BINARY_DIR}/llama" EXCLUDE_FROM_ALL)

add_library(ai-chat SHARED llama_binding/ai_chat.cpp)
target_include_directories(
    ai-chat
    PRIVATE
    "${LLAMA_SRC}"
    "${LLAMA_SRC}/common"
    "${LLAMA_SRC}/include"
    "${LLAMA_SRC}/ggml/include"
    "${LLAMA_SRC}/ggml/src"
    "${CMAKE_CURRENT_SOURCE_DIR}/llama_binding"
)
target_link_libraries(
    ai-chat
    PRIVATE
    llama
    llama-common
    ${android_log}
    ${android_lib}
)
