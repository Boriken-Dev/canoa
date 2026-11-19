#ifndef FLUTTER_PLUGIN_SPEECH_TO_TEXT_WINDOWS_PLUGIN_H_
#define FLUTTER_PLUGIN_SPEECH_TO_TEXT_WINDOWS_PLUGIN_H_

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>
#include <flutter/standard_method_codec.h>

#include <windows.h>

#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <string>

#include "sapi_dictation.h"

namespace speech_to_text_windows {

// The `speech_to_text_windows` channel over [sapi_dictation::Dictation]. SAPI runs on the dictation's worker thread; what it
// reports is handed to the platform thread through a message-only window, the one thread Flutter's channels may be used on.
class SpeechToTextWindowsPlugin : public flutter::Plugin {
 public:
  static void RegisterWithRegistrar(flutter::PluginRegistrarWindows* registrar);

  explicit SpeechToTextWindowsPlugin(std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel);
  ~SpeechToTextWindowsPlugin() override;

  SpeechToTextWindowsPlugin(const SpeechToTextWindowsPlugin&) = delete;
  SpeechToTextWindowsPlugin& operator=(const SpeechToTextWindowsPlugin&) = delete;

 private:
  using Result = std::shared_ptr<flutter::MethodResult<flutter::EncodableValue>>;

  void HandleMethodCall(const flutter::MethodCall<flutter::EncodableValue>& call,
                        std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);
  void OnPlatformThread(std::function<void()> task);
  void Drain();
  void Invoke(const char* method, const std::string& argument);
  static LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);

  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
  HWND window_ = nullptr;
  std::mutex mutex_;
  std::deque<std::function<void()>> queue_;
  std::wstring input_file_;
  // Last, so it is destroyed first: its worker still posts to the members above while it ends.
  std::unique_ptr<sapi_dictation::Dictation> dictation_;
};

}  // namespace speech_to_text_windows

#endif  // FLUTTER_PLUGIN_SPEECH_TO_TEXT_WINDOWS_PLUGIN_H_
