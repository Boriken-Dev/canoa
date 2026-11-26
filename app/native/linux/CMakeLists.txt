# Builds crates/canoa-ffi with cargo and hands the library to the runner, and adds to the executable the entry that makes it
# the host's own command when it starts itself. CANOA_WEB_DIST set builds the web app in.
cmake_minimum_required(VERSION 3.14)
project(canoa_native LANGUAGES CXX)

# The runner is generated in app/linux, so the workspace is two above it; the plugin's own directory is only a link.
get_filename_component(CANOA_WORKSPACE "${CMAKE_SOURCE_DIR}/../.." ABSOLUTE)
if(NOT EXISTS "${CANOA_WORKSPACE}/crates/canoa-ffi/Cargo.toml")
  message(FATAL_ERROR "canoa_native builds crates/canoa-ffi, which is not at ${CANOA_WORKSPACE}: generate the runner in app/")
endif()

# The program's name takes the flag that makes the application run as the host's own command: `--<program>-child`. It is the
# brand's, from the file BRAND_FILE names or the app's own, read as `canoa-ffi`'s build reads it: `program`, else the name in
# lower case without anything but letters and digits.
if(DEFINED ENV{BRAND_FILE} AND NOT "$ENV{BRAND_FILE}" STREQUAL "")
  file(TO_CMAKE_PATH "$ENV{BRAND_FILE}" CANOA_BRAND_FILE)
else()
  set(CANOA_BRAND_FILE "${CANOA_WORKSPACE}/app/brand.json")
endif()
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${CANOA_BRAND_FILE}")
file(READ "${CANOA_BRAND_FILE}" CANOA_BRAND_TEXT)
string(REGEX MATCH "\"program\"[ \t\r\n]*:[ \t\r\n]*\"([a-z][a-z0-9-]*)\"" CANOA_BRAND_PROGRAM "${CANOA_BRAND_TEXT}")
if(CMAKE_MATCH_1)
  set(CANOA_PROGRAM "${CMAKE_MATCH_1}")
else()
  string(REGEX MATCH "\"name\"[ \t\r\n]*:[ \t\r\n]*\"([^\"]+)\"" CANOA_BRAND_NAME "${CANOA_BRAND_TEXT}")
  string(TOLOWER "${CMAKE_MATCH_1}" CANOA_PROGRAM)
  string(REGEX REPLACE "[^a-z0-9]" "" CANOA_PROGRAM "${CANOA_PROGRAM}")
endif()

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
target_sources(${BINARY_NAME} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/child_entry.cc")
target_compile_definitions(${BINARY_NAME} PRIVATE
  "CANOA_PROGRAM=\"${CANOA_PROGRAM}\""
  "CANOA_CHILD_FLAG=\"--${CANOA_PROGRAM}-child\"")
target_link_libraries(${BINARY_NAME} PRIVATE ${CMAKE_DL_LIBS})

set(canoa_native_bundled_libraries
  "${CANOA_TARGET}/${CANOA_PROFILE_DIR}/libcanoa_ffi.so"
  PARENT_SCOPE
)
