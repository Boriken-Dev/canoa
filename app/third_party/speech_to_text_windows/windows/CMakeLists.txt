cmake_minimum_required(VERSION 3.14)
set(PROJECT_NAME "speech_to_text_windows")
project(${PROJECT_NAME} LANGUAGES CXX)

# This value is used when generating builds using this plugin, so it must not be changed
set(PLUGIN_NAME "speech_to_text_windows_plugin")

add_library(${PLUGIN_NAME} SHARED
  "include/speech_to_text_windows/speech_to_text_windows.h"
  "speech_to_text_windows.cpp"
  "speech_to_text_windows_plugin.cpp"
  "speech_to_text_windows_plugin.h"
  "sapi_dictation.cpp"
  "sapi_dictation.h"
  "paced_audio.h"
)

apply_standard_settings(${PLUGIN_NAME})

set_target_properties(${PLUGIN_NAME} PROPERTIES CXX_VISIBILITY_PRESET hidden)
target_compile_definitions(${PLUGIN_NAME} PRIVATE FLUTTER_PLUGIN_IMPL WIN32_LEAN_AND_MEAN NOMINMAX)
target_include_directories(${PLUGIN_NAME} INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_include_directories(${PLUGIN_NAME} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}")
target_link_libraries(${PLUGIN_NAME} PRIVATE flutter flutter_wrapper_plugin ole32 oleaut32 uuid sapi)

# List of absolute paths to libraries that should be bundled with the plugin
set(speech_to_text_windows_bundled_libraries "" PARENT_SCOPE)
