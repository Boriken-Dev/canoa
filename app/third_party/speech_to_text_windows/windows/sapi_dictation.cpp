#include "sapi_dictation.h"

#include <sapi.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <memory>

#include "paced_audio.h"

namespace sapi_dictation {

namespace {

// The recognizer reports a phrase as a SAPI result object; every event's payload must be released.
template <class T>
class Ptr {
 public:
  Ptr() = default;
  ~Ptr() { Reset(); }
  Ptr(const Ptr&) = delete;
  Ptr& operator=(const Ptr&) = delete;
  Ptr(Ptr&& other) noexcept : p_(other.p_) { other.p_ = nullptr; }
  Ptr& operator=(Ptr&& other) noexcept {
    if (this != &other) {
      Reset();
      p_ = other.p_;
      other.p_ = nullptr;
    }
    return *this;
  }
  T** Put() {
    Reset();
    return &p_;
  }
  void Adopt(T* p) {
    Reset();
    p_ = p;
  }
  T* Get() const { return p_; }
  T* operator->() const { return p_; }
  explicit operator bool() const { return p_ != nullptr; }
  void Reset() {
    if (p_) p_->Release();
    p_ = nullptr;
  }

 private:
  T* p_ = nullptr;
};

std::string Utf8(const wchar_t* wide) {
  const int size = WideCharToMultiByte(CP_UTF8, 0, wide, -1, nullptr, 0, nullptr, nullptr);
  if (size <= 1) return std::string();
  std::string out(static_cast<size_t>(size) - 1, '\0');
  WideCharToMultiByte(CP_UTF8, 0, wide, -1, &out[0], size, nullptr, nullptr);
  return out;
}

std::string Code(const char* base, HRESULT hr) {
  char text[64];
  snprintf(text, sizeof text, "%s_0x%08lx", base, static_cast<unsigned long>(hr));
  return text;
}

// An installed recognizer: its registry token and the languages it understands.
struct Installed {
  Ptr<ISpObjectToken> token;
  std::vector<Locale> languages;  // the first is the recognizer's own
  bool is_default = false;
};

// `Language` in a recognizer's attributes is hexadecimal language identifiers joined by `;` (`409;9`).
std::vector<Locale> LanguagesOf(ISpObjectToken* token) {
  std::vector<Locale> out;
  Ptr<ISpDataKey> attributes;
  if (FAILED(token->OpenKey(L"Attributes", attributes.Put()))) return out;
  LPWSTR raw = nullptr;
  if (FAILED(attributes->GetStringValue(L"Language", &raw)) || !raw) return out;
  std::wstring list(raw);
  CoTaskMemFree(raw);
  size_t at = 0;
  while (at <= list.size()) {
    size_t end = list.find(L';', at);
    if (end == std::wstring::npos) end = list.size();
    const std::wstring item = list.substr(at, end - at);
    at = end + 1;
    wchar_t* stop = nullptr;
    const unsigned long id = wcstoul(item.c_str(), &stop, 16);
    if (item.empty() || (stop && *stop)) continue;
    wchar_t tag[LOCALE_NAME_MAX_LENGTH];
    if (!LCIDToLocaleName(MAKELCID(static_cast<LANGID>(id), SORT_DEFAULT), tag, LOCALE_NAME_MAX_LENGTH, 0)) continue;
    wchar_t display[256] = L"";
    if (!GetLocaleInfoEx(tag, LOCALE_SLOCALIZEDDISPLAYNAME, display, 256)) wcscpy_s(display, tag);
    Locale locale{Utf8(tag), Utf8(display)};
    // The plugin's channel joins an id and a name with a colon.
    std::replace(locale.name.begin(), locale.name.end(), ':', ' ');
    out.push_back(locale);
  }
  return out;
}

HRESULT Enumerate(std::vector<Installed>* out) {
  Ptr<ISpObjectTokenCategory> category;
  HRESULT hr = CoCreateInstance(CLSID_SpObjectTokenCategory, nullptr, CLSCTX_ALL, IID_ISpObjectTokenCategory,
                                reinterpret_cast<void**>(category.Put()));
  if (FAILED(hr)) return hr;
  hr = category->SetId(SPCAT_RECOGNIZERS, FALSE);
  if (FAILED(hr)) return hr;
  LPWSTR default_id = nullptr;
  category->GetDefaultTokenId(&default_id);
  Ptr<IEnumSpObjectTokens> tokens;
  hr = category->EnumTokens(nullptr, nullptr, tokens.Put());
  if (FAILED(hr)) {
    if (default_id) CoTaskMemFree(default_id);
    return hr;
  }
  ULONG count = 0;
  tokens->GetCount(&count);
  for (ULONG i = 0; i < count; i++) {
    Installed item;
    if (FAILED(tokens->Item(i, item.token.Put()))) continue;
    item.languages = LanguagesOf(item.token.Get());
    LPWSTR id = nullptr;
    if (SUCCEEDED(item.token->GetId(&id)) && id) {
      item.is_default = default_id && wcscmp(id, default_id) == 0;
      CoTaskMemFree(id);
    }
    if (!item.languages.empty()) out->push_back(std::move(item));
  }
  if (default_id) CoTaskMemFree(default_id);
  std::stable_sort(out->begin(), out->end(), [](const Installed& a, const Installed& b) { return a.is_default && !b.is_default; });
  return S_OK;
}

std::string Lower(std::string text) {
  std::transform(text.begin(), text.end(), text.begin(), [](unsigned char c) { return static_cast<char>(tolower(c)); });
  return text;
}

// `en_US` and `EN-us` name `en-US`; `en` names any English recognizer.
bool Matches(const Locale& have, const std::string& want) {
  std::string wanted = Lower(want);
  std::replace(wanted.begin(), wanted.end(), '_', '-');
  const std::string tag = Lower(have.id);
  if (tag == wanted) return true;
  return wanted.find('-') == std::string::npos && tag.compare(0, wanted.size() + 1, wanted + "-") == 0;
}

void Clear(SPEVENT* event) {
  switch (event->elParamType) {
    case SPET_LPARAM_IS_POINTER:
    case SPET_LPARAM_IS_STRING:
      if (event->lParam) CoTaskMemFree(reinterpret_cast<void*>(event->lParam));
      break;
    case SPET_LPARAM_IS_TOKEN:
    case SPET_LPARAM_IS_OBJECT:
      if (event->lParam) reinterpret_cast<IUnknown*>(event->lParam)->Release();
      break;
    default:
      break;
  }
  event->lParam = 0;
}

std::string TextOf(ISpRecoResult* result) {
  LPWSTR text = nullptr;
  if (FAILED(result->GetText(static_cast<ULONG>(SP_GETWHOLEPHRASE), static_cast<ULONG>(SP_GETWHOLEPHRASE), TRUE, &text, nullptr)) || !text) return std::string();
  std::string out = Utf8(text);
  CoTaskMemFree(text);
  return out;
}

std::string JoinPhrases(const std::vector<std::string>& phrases) {
  std::string out;
  for (const std::string& phrase : phrases) {
    if (phrase.empty()) continue;
    if (!out.empty()) out += ' ';
    out += phrase;
  }
  return out;
}

}  // namespace

struct Dictation::Session {
  Ptr<ISpRecognizer> recognizer;
  Ptr<ISpRecoContext> context;
  Ptr<ISpRecoGrammar> grammar;
  Ptr<ISpAudio> audio;
  HANDLE notify = nullptr;
  std::vector<std::string> phrases;  // each phrase the recognizer has finished
  bool finishing = false;
  bool pending_phrase_ended = false;
  bool stream_ended = false;
  std::chrono::steady_clock::time_point deadline;
};

Dictation::Dictation(Sink sink, int finish_ms)
    : sink_(std::move(sink)), finish_ms_(finish_ms), wake_(CreateEventW(nullptr, FALSE, FALSE, nullptr)) {
  worker_ = std::thread([this] { Run(); });
}

Dictation::~Dictation() {
  {
    std::lock_guard<std::mutex> lock(mutex_);
    quit_ = true;
  }
  SetEvent(wake_);
  if (worker_.joinable()) worker_.join();
  CloseHandle(wake_);
}

void Dictation::Post(std::function<void()> task) {
  {
    std::lock_guard<std::mutex> lock(mutex_);
    tasks_.push_back(std::move(task));
  }
  SetEvent(wake_);
}

void Dictation::Initialize(Done done) {
  Post([this, done] {
    std::vector<Installed> installed;
    const HRESULT hr = Enumerate(&installed);
    if (FAILED(hr)) {
      const std::string code = Code("error_sapi", hr);
      sink_.error(code, true);
      done(false, code);
    } else if (installed.empty()) {
      sink_.error("error_no_recognizer", true);
      done(false, "error_no_recognizer");
    } else {
      done(true, "");
    }
  });
}

void Dictation::Locales(std::function<void(std::vector<Locale>)> done) {
  Post([done] {
    std::vector<Installed> installed;
    std::vector<Locale> out;
    if (SUCCEEDED(Enumerate(&installed))) {
      for (const Installed& item : installed) {
        for (const Locale& locale : item.languages) {
          const bool seen = std::any_of(out.begin(), out.end(), [&](const Locale& l) { return l.id == locale.id; });
          if (!seen) out.push_back(locale);
        }
      }
    }
    done(std::move(out));
  });
}

void Dictation::Start(StartOptions options, Done done) {
  Post([this, options, done] {
    delete session_;
    session_ = nullptr;
    std::string code;
    if (!CreateSession(options, &code)) {
      delete session_;
      session_ = nullptr;
      sink_.error(code, true);
      done(false, code);
      return;
    }
    sink_.status("listening");
    done(true, "");
  });
}

void Dictation::Stop() {
  Post([this] {
    if (!session_) {
      sink_.status("notListening");
      sink_.status("done");
      return;
    }
    if (session_->finishing) return;
    Pump();  // what the recognizer already holds is reported before the stop is asked for
    if (!session_) return;
    session_->finishing = true;
    session_->deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(finish_ms_);
    // The recognizer finishes the phrase it holds and reports it, then the stream ends.
    session_->recognizer->SetRecoState(SPRST_INACTIVE);
  });
}

void Dictation::Cancel() {
  Post([this] {
    delete session_;
    session_ = nullptr;
    sink_.status("notListening");
  });
}

bool Dictation::CreateSession(const StartOptions& options, std::string* code) {
  session_ = new Session();
  Session& s = *session_;
  HRESULT hr = CoCreateInstance(CLSID_SpInprocRecognizer, nullptr, CLSCTX_INPROC_SERVER, IID_ISpRecognizer,
                                reinterpret_cast<void**>(s.recognizer.Put()));
  if (FAILED(hr)) {
    *code = Code("error_no_recognizer", hr);
    return false;
  }
  if (!options.locale.empty()) {
    std::vector<Installed> installed;
    Enumerate(&installed);
    const Installed* chosen = nullptr;
    for (const Installed& item : installed) {
      if (std::any_of(item.languages.begin(), item.languages.end(), [&](const Locale& l) { return Matches(l, options.locale); })) {
        chosen = &item;
        break;
      }
    }
    if (!chosen) {
      *code = "error_language_unavailable";
      return false;
    }
    hr = s.recognizer->SetRecognizer(chosen->token.Get());
    if (FAILED(hr)) {
      *code = Code("error_language_unavailable", hr);
      return false;
    }
  }
  if (!options.input_file.empty()) {
    s.audio.Adopt(PacedAudio::Open(options.input_file.c_str()));
    if (!s.audio) {
      *code = "error_audio_file";
      return false;
    }
    hr = s.recognizer->SetInput(s.audio.Get(), TRUE);
  } else {
    hr = s.recognizer->SetInput(nullptr, TRUE);  // the default microphone
  }
  if (FAILED(hr)) {
    *code = Code("error_audio", hr);
    return false;
  }
  hr = s.recognizer->CreateRecoContext(s.context.Put());
  if (FAILED(hr)) {
    *code = Code("error_sapi", hr);
    return false;
  }
  // Without an interest, a context queues only the final phrase, and no hypothesis of the phrase being spoken.
  const ULONGLONG events = SPFEI(SPEI_RECOGNITION) | SPFEI(SPEI_HYPOTHESIS) | SPFEI(SPEI_END_SR_STREAM);
  hr = s.context->SetInterest(events, events);
  if (SUCCEEDED(hr)) hr = s.context->SetNotifyWin32Event();
  if (FAILED(hr)) {
    *code = Code("error_sapi", hr);
    return false;
  }
  s.notify = s.context->GetNotifyEventHandle();
  hr = s.context->CreateGrammar(0, s.grammar.Put());
  if (SUCCEEDED(hr)) hr = s.grammar->LoadDictation(nullptr, SPLO_STATIC);
  if (FAILED(hr)) {
    *code = Code("error_no_recognizer", hr);
    return false;
  }
  hr = s.recognizer->SetRecoState(SPRST_ACTIVE);
  if (SUCCEEDED(hr)) hr = s.grammar->SetDictationState(SPRS_ACTIVE);
  if (FAILED(hr)) {
    // No microphone, or one the system will not open.
    *code = Code("error_audio", hr);
    return false;
  }
  return true;
}

bool Dictation::DrainTasks() {
  for (;;) {
    std::function<void()> task;
    {
      std::lock_guard<std::mutex> lock(mutex_);
      if (quit_) return false;
      if (tasks_.empty()) return true;
      task = std::move(tasks_.front());
      tasks_.pop_front();
    }
    task();
  }
}

void Dictation::Pump() {
  Session& s = *session_;
  SPEVENT event;
  ULONG fetched = 0;
  while (SUCCEEDED(s.context->GetEvents(1, &event, &fetched)) && fetched == 1) {
    ISpRecoResult* result = (event.elParamType == SPET_LPARAM_IS_OBJECT) ? reinterpret_cast<ISpRecoResult*>(event.lParam) : nullptr;
    switch (event.eEventId) {
      case SPEI_HYPOTHESIS:
        if (result && !s.finishing) {
          std::vector<std::string> words = s.phrases;
          words.push_back(TextOf(result));
          sink_.text(JoinPhrases(words), false);
        }
        break;
      case SPEI_RECOGNITION:
        if (result) {
          s.phrases.push_back(TextOf(result));
          if (s.finishing) {
            s.pending_phrase_ended = true;
          } else {
            sink_.text(JoinPhrases(s.phrases), false);
          }
        }
        break;
      case SPEI_END_SR_STREAM:
        s.stream_ended = true;
        // The stream ending while still listening is the audio going away, not a person stopping.
        if (!s.finishing && FAILED(static_cast<HRESULT>(event.lParam))) {
          Clear(&event);
          Fail("error_audio");
          return;
        }
        break;
      default:
        break;
    }
    Clear(&event);
    if (!session_) return;
  }
  if (s.finishing && (s.stream_ended || s.pending_phrase_ended || std::chrono::steady_clock::now() >= s.deadline)) Finish();
}

void Dictation::Finish() {
  const std::string text = JoinPhrases(session_->phrases);
  delete session_;
  session_ = nullptr;
  if (!text.empty()) sink_.text(text, true);
  sink_.status("notListening");
  sink_.status("done");
}

void Dictation::Fail(const std::string& code) {
  const std::string text = JoinPhrases(session_->phrases);
  delete session_;
  session_ = nullptr;
  sink_.error(code, true);
  if (!text.empty()) sink_.text(text, true);
  sink_.status("notListening");
  sink_.status("done");
}

void Dictation::Run() {
  CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  for (;;) {
    HANDLE handles[2] = {wake_, session_ ? session_->notify : nullptr};
    const DWORD count = session_ ? 2 : 1;
    DWORD timeout = INFINITE;
    if (session_ && session_->finishing) {
      const auto left = std::chrono::duration_cast<std::chrono::milliseconds>(session_->deadline - std::chrono::steady_clock::now());
      timeout = left.count() > 0 ? static_cast<DWORD>(left.count()) : 0;
    }
    WaitForMultipleObjects(count, handles, FALSE, timeout);
    if (!DrainTasks()) break;
    if (session_) Pump();
  }
  delete session_;
  session_ = nullptr;
  CoUninitialize();
}

}  // namespace sapi_dictation
