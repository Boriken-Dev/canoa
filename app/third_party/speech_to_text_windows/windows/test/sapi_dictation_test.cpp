// Dictation against the installed Windows desktop recognizer, fed a recorded WAV in real time instead of the microphone.
//
//   sapi_dictation_test                       run the tests
//   sapi_dictation_test --make-wav OUT TEXT   write TEXT, spoken by the system's voice, as a 16 kHz WAV (SAPI XML allowed)
//
// The recordings are made by the system's own text-to-speech voice at run time, so nothing binary is stored. A machine with no
// English recognizer or voice cannot run the speech tests and says so.
#include <windows.h>
#include <sapi.h>

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>

#include "../sapi_dictation.h"

using sapi_dictation::Dictation;
using sapi_dictation::Sink;
using sapi_dictation::StartOptions;

namespace {

int failures = 0;
int checks = 0;

void Check(bool ok, const std::string& what) {
  checks++;
  if (!ok) failures++;
  printf("  %s: %s\n", ok ? "ok  " : "FAIL", what.c_str());
}

std::string Lower(std::string text) {
  std::transform(text.begin(), text.end(), text.begin(), [](unsigned char c) { return static_cast<char>(tolower(c)); });
  return text;
}

std::wstring Wide(const std::string& s) {
  std::wstring out(s.size() + 1, L'\0');
  const int n = MultiByteToWideChar(CP_UTF8, 0, s.c_str(), -1, &out[0], static_cast<int>(out.size()));
  out.resize(n > 0 ? n - 1 : 0);
  return out;
}

// Speaks [text] (SAPI XML, so `<silence msec="1500"/>` works) into a 16 kHz mono WAV.
bool MakeWav(const std::wstring& path, const std::wstring& text) {
  ISpVoice* voice = nullptr;
  ISpStream* stream = nullptr;
  if (FAILED(CoCreateInstance(CLSID_SpVoice, nullptr, CLSCTX_ALL, IID_ISpVoice, reinterpret_cast<void**>(&voice)))) return false;
  CoCreateInstance(CLSID_SpStream, nullptr, CLSCTX_ALL, IID_ISpStream, reinterpret_cast<void**>(&stream));
  WAVEFORMATEX format = {WAVE_FORMAT_PCM, 1, 16000, 32000, 2, 16, 0};
  bool ok = stream && SUCCEEDED(stream->BindToFile(path.c_str(), SPFM_CREATE_ALWAYS, &SPDFID_WaveFormatEx, &format, 0));
  ok = ok && SUCCEEDED(voice->SetOutput(stream, TRUE)) && SUCCEEDED(voice->Speak(text.c_str(), SPF_IS_XML, nullptr));
  if (ok) voice->WaitUntilDone(INFINITE);
  if (stream) {
    stream->Close();
    stream->Release();
  }
  voice->Release();
  return ok;
}

// Silence of [seconds] as a WAV, written by hand.
bool MakeSilence(const std::wstring& path, int seconds) {
  const uint32_t bytes = 32000u * seconds;
  std::vector<unsigned char> wav(44 + bytes, 0);
  auto put32 = [&](size_t at, uint32_t v) { memcpy(&wav[at], &v, 4); };
  auto put16 = [&](size_t at, uint16_t v) { memcpy(&wav[at], &v, 2); };
  memcpy(&wav[0], "RIFF", 4);
  put32(4, 36 + bytes);
  memcpy(&wav[8], "WAVEfmt ", 8);
  put32(16, 16);
  put16(20, 1);
  put16(22, 1);
  put32(24, 16000);
  put32(28, 32000);
  put16(32, 2);
  put16(34, 16);
  memcpy(&wav[36], "data", 4);
  put32(40, bytes);
  HANDLE file = CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return false;
  DWORD written = 0;
  const bool ok = WriteFile(file, wav.data(), static_cast<DWORD>(wav.size()), &written, nullptr) && written == wav.size();
  CloseHandle(file);
  return ok;
}

// Everything a dictation reports, in order, with a way to wait for it.
struct Report {
  std::mutex mutex;
  std::condition_variable changed;
  std::vector<std::string> partials;
  std::vector<std::string> finals;
  std::vector<std::string> statuses;
  std::vector<std::string> errors;
  std::chrono::steady_clock::time_point started = std::chrono::steady_clock::now();
  long long done_at_ms = -1;

  Sink MakeSink() {
    Sink sink;
    sink.text = [this](const std::string& text, bool final) {
      std::lock_guard<std::mutex> lock(mutex);
      (final ? finals : partials).push_back(text);
      changed.notify_all();
    };
    sink.status = [this](const std::string& status) {
      std::lock_guard<std::mutex> lock(mutex);
      statuses.push_back(status);
      if (status == "done") done_at_ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - started).count();
      changed.notify_all();
    };
    sink.error = [this](const std::string& code, bool) {
      std::lock_guard<std::mutex> lock(mutex);
      errors.push_back(code);
      changed.notify_all();
    };
    return sink;
  }

  template <class Pred>
  bool Wait(int ms, Pred pred) {
    std::unique_lock<std::mutex> lock(mutex);
    return changed.wait_for(lock, std::chrono::milliseconds(ms), pred);
  }
  void Mark() {
    std::lock_guard<std::mutex> lock(mutex);
    started = std::chrono::steady_clock::now();
    done_at_ms = -1;
  }
};

bool Contains(const std::string& text, const char* word) { return Lower(text).find(word) != std::string::npos; }

struct Started {
  bool ok = false;
  std::string code;
};

Started StartAndWait(Dictation& dictation, StartOptions options) {
  Started out;
  std::mutex m;
  std::condition_variable cv;
  bool called = false;
  dictation.Start(options, [&](bool ok, const std::string& code) {
    std::lock_guard<std::mutex> lock(m);
    out.ok = ok;
    out.code = code;
    called = true;
    cv.notify_all();
  });
  std::unique_lock<std::mutex> lock(m);
  cv.wait_for(lock, std::chrono::seconds(20), [&] { return called; });
  return out;
}

std::wstring Temp(const wchar_t* name) {
  wchar_t dir[MAX_PATH];
  GetTempPathW(MAX_PATH, dir);
  return std::wstring(dir) + L"sapi_dictation_test_" + name;
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
  CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  if (argc >= 4 && std::wstring(argv[1]) == L"--make-wav") return MakeWav(argv[2], argv[3]) ? 0 : 1;

  const std::wstring sentence = Temp(L"sentence.wav");
  const std::wstring two = Temp(L"two_phrases.wav");
  const std::wstring quiet = Temp(L"silence.wav");
  const bool voices =
      MakeWav(sentence, L"hello world this is a test of dictation") &&
      MakeWav(two, L"open the door. <silence msec=\"1800\"/> close the window.") && MakeSilence(quiet, 3);
  if (!voices) {
    printf("SKIP: no text-to-speech voice to record the test speech with\n");
    return 0;
  }

  {
    printf("locales and initialize\n");
    Report report;
    Dictation dictation(report.MakeSink());
    std::mutex m;
    std::condition_variable cv;
    bool called = false, ok = false;
    dictation.Initialize([&](bool o, const std::string&) {
      std::lock_guard<std::mutex> lock(m);
      ok = o;
      called = true;
      cv.notify_all();
    });
    {
      std::unique_lock<std::mutex> lock(m);
      cv.wait_for(lock, std::chrono::seconds(10), [&] { return called; });
    }
    Check(called && ok, "a recognizer is installed");
    if (!ok) {
      printf("SKIP: no speech recognizer installed\n");
      return 0;
    }
    std::vector<sapi_dictation::Locale> locales;
    bool got = false;
    dictation.Locales([&](std::vector<sapi_dictation::Locale> l) {
      std::lock_guard<std::mutex> lock(m);
      locales = std::move(l);
      got = true;
      cv.notify_all();
    });
    {
      std::unique_lock<std::mutex> lock(m);
      cv.wait_for(lock, std::chrono::seconds(10), [&] { return got; });
    }
    Check(!locales.empty(), "locales come from the installed recognizers");
    for (const auto& l : locales) printf("    %s : %s\n", l.id.c_str(), l.name.c_str());
    Check(locales.size() > 0 && locales[0].id.find('-') != std::string::npos && locales[0].name.find('(') != std::string::npos,
          "a locale has a language tag and a display name");

    printf("a sentence, stopped after it was heard\n");
    report.Mark();
    StartOptions options;
    options.input_file = sentence;
    Started started = StartAndWait(dictation, options);
    Check(started.ok, "listening starts on the recorded speech (" + started.code + ")");
    Check(report.Wait(2000, [&] { return !report.statuses.empty() && report.statuses[0] == "listening"; }), "the sink hears `listening`");
    Check(report.Wait(15000, [&] { return !report.partials.empty() && Lower(report.partials.back()).find("dictation") != std::string::npos; }),
          "the words are heard while the speech plays");
    dictation.Stop();
    Check(report.Wait(5000, [&] { return report.done_at_ms >= 0; }), "stopping ends with `done`");
    {
      std::lock_guard<std::mutex> lock(report.mutex);
      Check(report.partials.size() >= 3, "words arrive as they are recognized, not only at the end (" + std::to_string(report.partials.size()) + " updates)");
      Check(report.finals.size() == 1, "one final text");
      Check(!report.finals.empty() && Lower(report.finals[0]).find("hello world this is a test of dictation") == 0, "the final text is the sentence: \"" + (report.finals.empty() ? "" : report.finals[0]) + "\"");
      const auto& s = report.statuses;
      Check(s.size() == 3 && s[0] == "listening" && s[1] == "notListening" && s[2] == "done", "statuses are listening, notListening, done");
      Check(report.errors.empty(), "no error");
    }

    printf("stopped in the middle of the sentence\n");
    {
      Report mid;
      Dictation d2(mid.MakeSink());
      mid.Mark();
      Started s2 = StartAndWait(d2, options);
      Check(s2.ok, "listening starts");
      Check(mid.Wait(15000, [&] { return mid.partials.size() >= 3; }), "the first words are heard");
      d2.Stop();
      Check(mid.Wait(5000, [&] { return mid.done_at_ms >= 0; }), "stopping ends with `done`");
      std::lock_guard<std::mutex> lock(mid.mutex);
      Check(mid.finals.size() == 1 && !mid.finals[0].empty(), "the phrase being spoken is not lost: \"" + (mid.finals.empty() ? "" : mid.finals[0]) + "\"");
      Check(Contains(mid.finals.empty() ? "" : mid.finals[0], "hello"), "and it starts the sentence");
      printf("    stop took %lld ms in all\n", mid.done_at_ms);
    }

    printf("a second dictation on the same service\n");
    {
      Report again;
      Dictation d3(again.MakeSink());
      for (int round = 1; round <= 2; round++) {
        again.Mark();
        {
          std::lock_guard<std::mutex> lock(again.mutex);
          again.partials.clear();
          again.finals.clear();
          again.statuses.clear();
          again.done_at_ms = -1;
        }
        Started s3 = StartAndWait(d3, options);
        Check(s3.ok, "round " + std::to_string(round) + " starts");
        Check(again.Wait(15000, [&] { return !again.partials.empty() && Contains(again.partials.back(), "dictation"); }), "round " + std::to_string(round) + " hears the sentence");
        d3.Stop();
        Check(again.Wait(5000, [&] { return again.done_at_ms >= 0; }), "round " + std::to_string(round) + " ends");
        std::lock_guard<std::mutex> lock(again.mutex);
        Check(again.finals.size() == 1 && Contains(again.finals[0], "hello world"), "round " + std::to_string(round) + " has its own final text");
      }
    }

    printf("two phrases with a pause between them\n");
    {
      Report p;
      Dictation d4(p.MakeSink());
      StartOptions o2;
      o2.input_file = two;
      Started s4 = StartAndWait(d4, o2);
      Check(s4.ok, "listening starts");
      Check(p.Wait(20000, [&] { return !p.partials.empty() && Contains(p.partials.back(), "window"); }), "the second phrase is heard too");
      d4.Stop();
      Check(p.Wait(5000, [&] { return p.done_at_ms >= 0; }), "stopping ends with `done`");
      std::lock_guard<std::mutex> lock(p.mutex);
      Check(p.finals.size() == 1 && Contains(p.finals[0], "door") && Contains(p.finals[0], "window"), "the final text holds both phrases: \"" + (p.finals.empty() ? "" : p.finals[0]) + "\"");
    }

    printf("silence\n");
    {
      Report q;
      Dictation d5(q.MakeSink());
      StartOptions o3;
      o3.input_file = quiet;
      Started s5 = StartAndWait(d5, o3);
      Check(s5.ok, "listening starts");
      Sleep(800);
      q.Mark();
      d5.Stop();
      Check(q.Wait(5000, [&] { return q.done_at_ms >= 0; }), "stopping ends with `done` though nothing was said");
      std::lock_guard<std::mutex> lock(q.mutex);
      Check(q.finals.empty(), "and no text is made up");
      printf("    stop took %lld ms\n", q.done_at_ms);
    }

    printf("cancel\n");
    {
      Report c;
      Dictation d6(c.MakeSink());
      Started s6 = StartAndWait(d6, options);
      Check(s6.ok, "listening starts");
      Check(c.Wait(15000, [&] { return !c.partials.empty(); }), "words are heard");
      d6.Cancel();
      Check(c.Wait(3000, [&] { return !c.statuses.empty() && c.statuses.back() == "notListening"; }), "cancel says `notListening`");
      Sleep(500);
      std::lock_guard<std::mutex> lock(c.mutex);
      Check(c.finals.empty(), "cancel sends no final text");
    }

    printf("stop with nothing running\n");
    {
      Report i;
      Dictation d7(i.MakeSink());
      d7.Stop();
      Check(i.Wait(2000, [&] { return i.done_at_ms >= 0; }), "an idle stop still ends with `done`");
    }

    printf("a language no recognizer has\n");
    {
      Report l;
      Dictation d8(l.MakeSink());
      StartOptions o4;
      o4.input_file = sentence;
      o4.locale = "xx-XX";
      Started s8 = StartAndWait(d8, o4);
      Check(!s8.ok && s8.code == "error_language_unavailable", "starting is refused with the reason: " + s8.code);
      Check(l.Wait(1000, [&] { return !l.errors.empty(); }) && l.errors[0] == "error_language_unavailable", "and the sink is told");
    }

    printf("the language of an installed recognizer\n");
    {
      Report l;
      Dictation d9(l.MakeSink());
      StartOptions o5;
      o5.input_file = sentence;
      o5.locale = locales.empty() ? "en-US" : locales[0].id;
      Started s9 = StartAndWait(d9, o5);
      Check(s9.ok, "starting with the locale " + o5.locale + " works (" + s9.code + ")");
      l.Wait(15000, [&] { return !l.partials.empty(); });
      d9.Cancel();
    }

    printf("a recording that is not there\n");
    {
      Report l;
      Dictation d10(l.MakeSink());
      StartOptions o6;
      o6.input_file = Temp(L"missing.wav");
      Started s10 = StartAndWait(d10, o6);
      Check(!s10.ok && s10.code == "error_audio_file", "starting is refused with the reason: " + s10.code);
      Check(l.Wait(1000, [&] { return !l.errors.empty(); }), "and the sink is told");
    }
  }

  DeleteFileW(sentence.c_str());
  DeleteFileW(two.c_str());
  DeleteFileW(quiet.c_str());
  printf("%d checks, %d failed\n", checks, failures);
  return failures == 0 ? 0 : 1;
}
