# Builds crates/canoa-ffi with cargo and hands the DLL to the runner, and adds to the executable the entry that makes it the
# host's own command when it starts itself. CANOA_WEB_DIST set builds the web app in.
cmake_minimum_required(VERSION 3.14)
project(canoa_native LANGUAGES CXX)

# The plugin is reached through a link under the runner's ephemeral directory; the workspace is above where it really is.
get_filename_component(CANOA_NATIVE_DIR "${CMAKE_CURRENT_SOURCE_DIR}" REALPATH)
get_filename_component(CANOA_WORKSPACE "${CANOA_NATIVE_DIR}/../../.." ABSOLUTE)

find_program(CANOA_CARGO cargo HINTS "$ENV{USERPROFILE}/.cargo/bin" REQUIRED)
if(DEFINED ENV{CARGO_TARGET_DIR})
  file(TO_CMAKE_PATH "$ENV{CARGO_TARGET_DIR}" CANOA_TARGET)
else()
  set(CANOA_TARGET "${CANOA_WORKSPACE}/target")
endif()
set(CANOA_FEATURES "")
if(DEFINED ENV{CANOA_WEB_DIST})
  set(CANOA_FEATURES "--features=web")
endif()

add_custom_target(canoa_ffi_build ALL
  COMMAND "${CANOA_CARGO}" build
    --manifest-path "${CANOA_WORKSPACE}/Cargo.toml"
    --target-dir "${CANOA_TARGET}"
    -p canoa-ffi --lib
    ${CANOA_FEATURES}
    "$<$<NOT:$<CONFIG:Debug>>:--release>"
  COMMAND_EXPAND_LISTS
  VERBATIM
)
add_dependencies(${BINARY_NAME} canoa_ffi_build)

# Before the runner's own code, whichever runner `flutter create` wrote.
target_sources(${BINARY_NAME} PRIVATE "${CANOA_NATIVE_DIR}/windows/child_entry.cpp")

set(canoa_native_bundled_libraries
  "${CANOA_TARGET}/$<IF:$<CONFIG:Debug>,debug,release>/canoa_ffi.dll"
  PARENT_SCOPE
)
