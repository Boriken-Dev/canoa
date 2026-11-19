// Dictation over the Windows desktop speech recognizer (SAPI 5), with no dependency on Flutter.
//
// Every SAPI object lives on one worker thread with its own COM apartment, and every callback of a [Sink] runs there, so a
// caller never touches SAPI from its own thread and never blocks on a recognizer.
#pragma once

#include <windows.h>

#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace sapi_dictation {

// One language an installed recognizer understands: `id` is a BCP 47 tag (`en-US`), `name` the language's display name.
struct Locale {
  std::string id, name;
};

// Where a dictation reports. Called on the worker thread, in order.
struct Sink {
  // The words of the session so far: each phrase heard, joined. `final` is true once, when the session has ended.
  std::function<void(const std::string& text, bool final)> text;
  // `listening` once the recognizer is taking audio; `notListening`, then `done` when the session has ended.
  std::function<void(const std::string& status)> status;
  // A failure: `code` names it (`error_no_recognizer`, `error_language_unavailable`, `error_audio`, ...).
  std::function<void(const std::string& code, bool permanent)> error;
};

struct StartOptions {
  // A BCP 47 tag from [Dictation::Locales]; empty is the system's own recognizer.
  std::string locale;
  // A PCM WAV file played in real time in place of the microphone; empty is the default microphone.
  std::wstring input_file;
};

using Done = std::function<void(bool ok, const std::string& code)>;

class Dictation {
 public:
  // [finish_ms] bounds how long a stopped session waits for the phrase still in the recognizer.
  explicit Dictation(Sink sink, int finish_ms = 1500);
  // Ends a session in progress without a result and joins the worker.
  ~Dictation();
  Dictation(const Dictation&) = delete;
  Dictation& operator=(const Dictation&) = delete;

  // Checks that a recognizer is installed. [done] is called with `error_no_recognizer` when none is.
  void Initialize(Done done);
  void Locales(std::function<void(std::vector<Locale>)> done);
  // Starts listening. [done] is called once the recognizer takes audio, or with the reason it cannot (which is also sent to
  // the sink's `error`). A session already running is ended first, without a result.
  void Start(StartOptions options, Done done);
  // Ends the audio and lets the recognizer finish the phrase it is in: the sink then gets the final text and the statuses.
  void Stop();
  // Ends the session now; no text is sent.
  void Cancel();

 private:
  struct Session;
  void Post(std::function<void()> task);
  void Run();
  bool DrainTasks();
  void Pump();
  void Finish();
  void Fail(const std::string& code);
  bool CreateSession(const StartOptions& options, std::string* code);

  Sink sink_;
  const int finish_ms_;
  HANDLE wake_;
  std::mutex mutex_;
  std::deque<std::function<void()>> tasks_;
  bool quit_ = false;
  Session* session_ = nullptr;
  std::thread worker_;
};

}  // namespace sapi_dictation
