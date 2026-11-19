cmake_minimum_required(VERSION 3.14)
project(sapi_dictation_test LANGUAGES CXX)

# Standalone: no Flutter, only the Windows speech API. See the plugin's README for the commands.
add_executable(sapi_dictation_test sapi_dictation_test.cpp ../sapi_dictation.cpp)
target_compile_features(sapi_dictation_test PRIVATE cxx_std_17)
target_compile_definitions(sapi_dictation_test PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX UNICODE _UNICODE)
if(MSVC)
  target_compile_options(sapi_dictation_test PRIVATE /W4 /WX /EHsc)
endif()
target_link_libraries(sapi_dictation_test PRIVATE ole32 oleaut32 uuid sapi)
