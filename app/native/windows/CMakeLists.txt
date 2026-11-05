# Builds crates/canoa-ffi with cargo and hands the DLL to the runner, and adds to the executable the entry that makes it the
# host's own command when it starts itself. CANOA_WEB_DIST set builds the web app in.
cmake_minimum_required(VERSION 3.14)
project(canoa_native LANGUAGES CXX)

# The runner is generated in app/windows, so the workspace is two above it; the plugin's own directory is only a link.
get_filename_component(CANOA_WORKSPACE "${CMAKE_SOURCE_DIR}/../.." ABSOLUTE)
if(NOT EXISTS "${CANOA_WORKSPACE}/crates/canoa-ffi/Cargo.toml")
  message(FATAL_ERROR "canoa_native builds crates/canoa-ffi, which is not at ${CANOA_WORKSPACE}: generate the runner in app/")
endif()

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
target_sources(${BINARY_NAME} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/child_entry.cpp")

set(canoa_native_bundled_libraries
  "${CANOA_TARGET}/$<IF:$<CONFIG:Debug>,debug,release>/canoa_ffi.dll"
  PARENT_SCOPE
)
