cmake_minimum_required(VERSION 3.22)
project(streamax_jni LANGUAGES C)

# Path to the streamax-core source — adjust if you move it.
set(STREAMAX_CORE_INCLUDE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../streamax-core/include")

add_library(streamax_jni SHARED streamax_jni.c)
target_include_directories(streamax_jni PRIVATE ${STREAMAX_CORE_INCLUDE})

# Each ABI's libstreamax_core.so lives in src/main/jniLibs/<abi>/.
# CMake's IMPORTED target lets us link against it without copying.
add_library(streamax_core SHARED IMPORTED)
set_target_properties(streamax_core PROPERTIES
    IMPORTED_LOCATION
    "${CMAKE_CURRENT_SOURCE_DIR}/../jniLibs/${ANDROID_ABI}/libstreamax_core.so")

target_link_libraries(streamax_jni streamax_core log)
