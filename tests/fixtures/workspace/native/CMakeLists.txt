cmake_minimum_required(VERSION 3.16)
project(fixture_native LANGUAGES C)

add_library(fixture_native STATIC native.c)
set_target_properties(fixture_native PROPERTIES POSITION_INDEPENDENT_CODE ON)

# Preserve native debug information without leaking source or build paths.
if(MSVC)
    target_compile_options(fixture_native PRIVATE
        "/pathmap:${CMAKE_CURRENT_SOURCE_DIR}=/native"
        "/pathmap:${CMAKE_CURRENT_BINARY_DIR}=/native-build")
else()
    target_compile_options(fixture_native PRIVATE
        "-ffile-prefix-map=${CMAKE_CURRENT_SOURCE_DIR}=/native"
        "-ffile-prefix-map=${CMAKE_CURRENT_BINARY_DIR}=/native-build")
endif()
