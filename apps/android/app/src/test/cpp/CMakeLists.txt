cmake_minimum_required(VERSION 3.22.1)
project(nola_model_privacy_test LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# Only headers are required. Studio's bundled runtime can lack a development kit.
set(JDK "$ENV{JAVA_HOME}")
if(APPLE AND NOT EXISTS "${JDK}/include/jni.h")
    execute_process(COMMAND /usr/libexec/java_home -v 21 OUTPUT_VARIABLE JDK OUTPUT_STRIP_TRAILING_WHITESPACE)
endif()
find_path(JNI_INCLUDE jni.h HINTS "${JDK}/include" REQUIRED)
find_path(JNI_PLATFORM_INCLUDE jni_md.h HINTS "${JNI_INCLUDE}"
    PATH_SUFFIXES darwin linux win32 REQUIRED)
add_library(nola_privacy_test SHARED ../../main/cpp/model_privacy_jni.cpp ../../main/cpp/nola_core_jni.cpp)
target_include_directories(nola_privacy_test PRIVATE "${JNI_INCLUDE}" "${JNI_PLATFORM_INCLUDE}"
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../../../crates/nola-app/include")
find_library(CORE_PRIVACY NAMES nola_app nola_app.dll
    PATHS "${RUST_OUTPUT}" NO_DEFAULT_PATH REQUIRED)
target_link_libraries(nola_privacy_test PRIVATE "${CORE_PRIVACY}")
# The JVM loads this adapter by absolute path, including on multi-config generators.
set_target_properties(nola_privacy_test PROPERTIES LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/lib"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/lib")
if(WIN32)
    add_custom_command(TARGET nola_privacy_test POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${RUST_OUTPUT}/nola_app.dll" "$<TARGET_FILE_DIR:nola_privacy_test>")
endif()
