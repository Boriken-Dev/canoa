#include "speech_to_text_windows_plugin.h"

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>
#include <flutter/standard_method_codec.h>

#include <cstdio>
#include <string>
#include <vector>

namespace speech_to_text_windows {

namespace {

constexpr UINT kDrain = WM_APP + 0x51;
constexpr wchar_t kWindowClass[] = L"SpeechToTextWindowsPlugin";
constexpr int kPartialResult = 0;
constexpr int kFinalResult = 2;

std::string Escape(const std::string& text) {
  std::string out;
  for (const unsigned char c : text) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          char hex[8];
          snprintf(hex, sizeof hex, "\\u%04x", c);
          out += hex;
        } else {
          out += static_cast<char>(c);
        }
    }
  }
  return out;
}

// What speech_to_text decodes as a recognition result; the confidence is -1, which it reads as not reported.
std::string ResultJson(const std::string& text, bool final) {
  return "{\"alternates\":[{\"recognizedWords\":\"" + Escape(text) + "\",\"confidence\":-1}],\"resultType\":" +
         std::to_string(final ? kFinalResult : kPartialResult) + "}";
}

std::string ErrorJson(const std::string& code, bool permanent) {
  return "{\"errorMsg\":\"" + Escape(code) + "\",\"permanent\":" + (permanent ? "true" : "false") + "}";
}

std::wstring Wide(const std::string& utf8) {
  const int size = MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(), -1, nullptr, 0);
  if (size <= 1) return std::wstring();
  std::wstring out(static_cast<size_t>(size) - 1, L'\0');
  MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(), -1, &out[0], size);
  return out;
}

const flutter::EncodableValue* Field(const flutter::MethodCall<flutter::EncodableValue>& call, const char* name) {
  const auto* map = std::get_if<flutter::EncodableMap>(call.arguments());
  if (!map) return nullptr;
  const auto found = map->find(flutter::EncodableValue(name));
  return found == map->end() ? nullptr : &found->second;
}

}  // namespace

void SpeechToTextWindowsPlugin::RegisterWithRegistrar(flutter::PluginRegistrarWindows* registrar) {
  auto channel = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      registrar->messenger(), "speech_to_text_windows", &flutter::StandardMethodCodec::GetInstance());
  auto plugin = std::make_unique<SpeechToTextWindowsPlugin>(std::move(channel));
  auto* raw = plugin.get();
  raw->channel_->SetMethodCallHandler([raw](const auto& call, auto result) { raw->HandleMethodCall(call, std::move(result)); });
  registrar->AddPlugin(std::move(plugin));
}

SpeechToTextWindowsPlugin::SpeechToTextWindowsPlugin(std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel)
    : channel_(std::move(channel)) {
  HMODULE module = nullptr;
  GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                     reinterpret_cast<LPCWSTR>(&SpeechToTextWindowsPlugin::WindowProc), &module);
  WNDCLASSEXW window_class = {};
  window_class.cbSize = sizeof window_class;
  window_class.lpfnWndProc = &SpeechToTextWindowsPlugin::WindowProc;
  window_class.hInstance = module;
  window_class.lpszClassName = kWindowClass;
  RegisterClassExW(&window_class);  // a second engine in the process finds the class already there
  window_ = CreateWindowExW(0, kWindowClass, L"", 0, 0, 0, 0, 0, HWND_MESSAGE, nullptr, module, this);

  sapi_dictation::Sink sink;
  sink.text = [this](const std::string& text, bool final) {
    const std::string json = ResultJson(text, final);
    OnPlatformThread([this, json] { Invoke("textRecognition", json); });
  };
  sink.status = [this](const std::string& status) {
    OnPlatformThread([this, status] { Invoke("notifyStatus", status); });
  };
  sink.error = [this](const std::string& code, bool permanent) {
    const std::string json = ErrorJson(code, permanent);
    OnPlatformThread([this, json] { Invoke("notifyError", json); });
  };
  dictation_ = std::make_unique<sapi_dictation::Dictation>(std::move(sink));
}

SpeechToTextWindowsPlugin::~SpeechToTextWindowsPlugin() {
  dictation_.reset();
  if (window_) DestroyWindow(window_);
}

LRESULT CALLBACK SpeechToTextWindowsPlugin::WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(create->lpCreateParams));
  } else if (message == kDrain) {
    if (auto* self = reinterpret_cast<SpeechToTextWindowsPlugin*>(GetWindowLongPtrW(window, GWLP_USERDATA))) self->Drain();
    return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

void SpeechToTextWindowsPlugin::OnPlatformThread(std::function<void()> task) {
  {
    std::lock_guard<std::mutex> lock(mutex_);
    queue_.push_back(std::move(task));
  }
  PostMessageW(window_, kDrain, 0, 0);
}

void SpeechToTextWindowsPlugin::Drain() {
  for (;;) {
    std::function<void()> task;
    {
      std::lock_guard<std::mutex> lock(mutex_);
      if (queue_.empty()) return;
      task = std::move(queue_.front());
      queue_.pop_front();
    }
    task();
  }
}

void SpeechToTextWindowsPlugin::Invoke(const char* method, const std::string& argument) {
  channel_->InvokeMethod(method, std::make_unique<flutter::EncodableValue>(argument));
}

void SpeechToTextWindowsPlugin::HandleMethodCall(const flutter::MethodCall<flutter::EncodableValue>& call,
                                                 std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> unique_result) {
  const std::string& method = call.method_name();
  Result result = std::move(unique_result);

  if (method == "hasPermission") {
    // SAPI has no permission to ask for: a microphone the system will not open fails the listen, with its own error.
    result->Success(flutter::EncodableValue(true));
  } else if (method == "initialize") {
    // `inputFile` is a recorded WAV played in place of the microphone (tests).
    if (const auto* file = Field(call, "inputFile")) {
      if (const auto* path = std::get_if<std::string>(file)) input_file_ = Wide(*path);
    }
    dictation_->Initialize([this, result](bool ok, const std::string&) {
      OnPlatformThread([result, ok] { result->Success(flutter::EncodableValue(ok)); });
    });
  } else if (method == "listen") {
    sapi_dictation::StartOptions options;
    options.input_file = input_file_;
    if (const auto* locale = Field(call, "localeId")) {
      if (const auto* id = std::get_if<std::string>(locale)) options.locale = *id;
    }
    dictation_->Start(options, [this, result](bool ok, const std::string&) {
      OnPlatformThread([result, ok] { result->Success(flutter::EncodableValue(ok)); });
    });
  } else if (method == "stop") {
    dictation_->Stop();
    result->Success();
  } else if (method == "cancel") {
    dictation_->Cancel();
    result->Success();
  } else if (method == "locales") {
    dictation_->Locales([this, result](std::vector<sapi_dictation::Locale> locales) {
      flutter::EncodableList list;
      for (const auto& locale : locales) list.push_back(flutter::EncodableValue(locale.id + ":" + locale.name));
      OnPlatformThread([result, list] { result->Success(flutter::EncodableValue(list)); });
    });
  } else {
    result->NotImplemented();
  }
}

}  // namespace speech_to_text_windows
