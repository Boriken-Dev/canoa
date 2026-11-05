# Builds crates/canoa-ffi with cargo and hands the library to the runner, and adds to the executable the entry that makes it
# the host's own command when it starts itself. CANOA_WEB_DIST set builds the web app in.
cmake_minimum_required(VERSION 3.14)
project(canoa_native LANGUAGES CXX)

# The plugin is reached through a link under the runner's ephemeral directory; the workspace is above where it really is.
get_filename_component(CANOA_NATIVE_DIR "${CMAKE_CURRENT_SOURCE_DIR}" REALPATH)
get_filename_component(CANOA_WORKSPACE "${CANOA_NATIVE_DIR}/../../.." ABSOLUTE)

find_program(CANOA_CARGO cargo HINTS "$ENV{HOME}/.cargo/bin" REQUIRED)
if(DEFINED ENV{CARGO_TARGET_DIR})
  set(CANOA_TARGET "$ENV{CARGO_TARGET_DIR}")
else()
  set(CANOA_TARGET "${CANOA_WORKSPACE}/target")
endif()
set(CANOA_FEATURES "")
if(DEFINED ENV{CANOA_WEB_DIST})
  set(CANOA_FEATURES "--features=web")
endif()
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
  set(CANOA_PROFILE_FLAG "")
  set(CANOA_PROFILE_DIR "debug")
else()
  set(CANOA_PROFILE_FLAG "--release")
  set(CANOA_PROFILE_DIR "release")
endif()

add_custom_target(canoa_ffi_build ALL
  COMMAND "${CANOA_CARGO}" build
    --manifest-path "${CANOA_WORKSPACE}/Cargo.toml"
    --target-dir "${CANOA_TARGET}"
    -p canoa-ffi --lib
    ${CANOA_FEATURES}
    ${CANOA_PROFILE_FLAG}
  VERBATIM
)
add_dependencies(${BINARY_NAME} canoa_ffi_build)

# Before the runner's own code, whichever runner `flutter create` wrote.
target_sources(${BINARY_NAME} PRIVATE "${CANOA_NATIVE_DIR}/linux/child_entry.cc")
target_link_libraries(${BINARY_NAME} PRIVATE ${CMAKE_DL_LIBS})

set(canoa_native_bundled_libraries
  "${CANOA_TARGET}/${CANOA_PROFILE_DIR}/libcanoa_ffi.so"
  PARENT_SCOPE
)
