// A speech audio source that plays a PCM WAV file in real time and then silence, as a microphone does. A recognizer given
// a plain file stream reads it faster than real time and treats its end as the end of speech; this behaves like a device,
// so stopping it mid-phrase is the same situation as a person stopping mid-phrase.
#pragma once

#include <windows.h>
#include <sapi.h>

#include <atomic>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <vector>

namespace sapi_dictation {

class PacedAudio final : public ISpAudio {
 public:
  // Reads [path]; null when it cannot be read or is not 16-bit-style PCM WAV.
  static PacedAudio* Open(const wchar_t* path) {
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return nullptr;
    LARGE_INTEGER size;
    std::vector<uint8_t> raw;
    DWORD got = 0;
    bool ok = GetFileSizeEx(file, &size) && size.QuadPart < (1LL << 31);
    if (ok) {
      raw.resize(static_cast<size_t>(size.QuadPart));
      ok = ReadFile(file, raw.data(), static_cast<DWORD>(raw.size()), &got, nullptr) && got == raw.size();
    }
    CloseHandle(file);
    if (!ok || raw.size() < 12 || memcmp(raw.data(), "RIFF", 4) != 0 || memcmp(raw.data() + 8, "WAVE", 4) != 0) return nullptr;

    PacedAudio* audio = new PacedAudio();
    bool have_format = false, have_data = false;
    size_t at = 12;
    while (at + 8 <= raw.size()) {
      uint32_t length;
      memcpy(&length, raw.data() + at + 4, 4);
      const uint8_t* body = raw.data() + at + 8;
      const size_t room = raw.size() - (at + 8);
      if (length > room) length = static_cast<uint32_t>(room);
      if (memcmp(raw.data() + at, "fmt ", 4) == 0 && length >= 16) {
        memcpy(&audio->format_, body, 16);
        have_format = true;
      } else if (memcmp(raw.data() + at, "data", 4) == 0) {
        audio->data_.assign(body, body + length);
        have_data = true;
      }
      at += 8 + length + (length & 1);
    }
    if (!have_format || !have_data || audio->format_.wFormatTag != WAVE_FORMAT_PCM || audio->format_.nBlockAlign == 0 ||
        audio->format_.nAvgBytesPerSec == 0) {
      audio->Release();
      return nullptr;
    }
    audio->format_.cbSize = 0;
    audio->event_ = CreateEventW(nullptr, TRUE, TRUE, nullptr);
    return audio;
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** out) override {
    if (riid == IID_IUnknown || riid == IID_ISequentialStream || riid == IID_IStream || riid == IID_ISpStreamFormat ||
        riid == IID_ISpAudio) {
      *out = static_cast<ISpAudio*>(this);
      AddRef();
      return S_OK;
    }
    *out = nullptr;
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG left = --refs_;
    if (left == 0) delete this;
    return left;
  }

  // Blocks until at least one sample is due, like a device; returns nothing once the state is not running.
  STDMETHODIMP Read(void* buffer, ULONG wanted, ULONG* read) override {
    ULONG done = 0;
    while (done == 0) {
      {
        std::lock_guard<std::mutex> lock(mutex_);
        if (state_ != SPAS_RUN) break;
        const uint64_t due = DueLocked();
        if (due > position_) {
          uint64_t count = due - position_;
          if (count > wanted) count = wanted;
          count -= count % format_.nBlockAlign;
          if (count > 0) {
            auto* out = static_cast<uint8_t*>(buffer);
            for (uint64_t i = 0; i < count; i++) out[i] = position_ + i < data_.size() ? data_[static_cast<size_t>(position_ + i)] : 0;
            position_ += count;
            done = static_cast<ULONG>(count);
            continue;
          }
        }
      }
      Sleep(5);
    }
    if (read) *read = done;
    return S_OK;
  }
  STDMETHODIMP Write(const void*, ULONG, ULONG* written) override {
    if (written) *written = 0;
    return E_NOTIMPL;
  }
  // A device cannot seek; the position query (zero from the current place) is the one call engines make.
  STDMETHODIMP Seek(LARGE_INTEGER move, DWORD origin, ULARGE_INTEGER* position) override {
    if (origin != STREAM_SEEK_CUR || move.QuadPart != 0) return STG_E_INVALIDFUNCTION;
    std::lock_guard<std::mutex> lock(mutex_);
    if (position) position->QuadPart = position_;
    return S_OK;
  }
  STDMETHODIMP SetSize(ULARGE_INTEGER) override { return E_NOTIMPL; }
  STDMETHODIMP CopyTo(IStream*, ULARGE_INTEGER, ULARGE_INTEGER*, ULARGE_INTEGER*) override { return E_NOTIMPL; }
  STDMETHODIMP Commit(DWORD) override { return S_OK; }
  STDMETHODIMP Revert() override { return E_NOTIMPL; }
  STDMETHODIMP LockRegion(ULARGE_INTEGER, ULARGE_INTEGER, DWORD) override { return E_NOTIMPL; }
  STDMETHODIMP UnlockRegion(ULARGE_INTEGER, ULARGE_INTEGER, DWORD) override { return E_NOTIMPL; }
  STDMETHODIMP Stat(STATSTG* stat, DWORD) override {
    if (stat) memset(stat, 0, sizeof *stat);
    return E_NOTIMPL;
  }
  STDMETHODIMP Clone(IStream**) override { return E_NOTIMPL; }

  STDMETHODIMP GetFormat(GUID* id, WAVEFORMATEX** out) override {
    if (id) *id = SPDFID_WaveFormatEx;
    if (!out) return E_POINTER;
    *out = static_cast<WAVEFORMATEX*>(CoTaskMemAlloc(sizeof(WAVEFORMATEX)));
    if (!*out) return E_OUTOFMEMORY;
    **out = format_;
    return S_OK;
  }

  STDMETHODIMP SetState(SPAUDIOSTATE state, ULONGLONG) override {
    std::lock_guard<std::mutex> lock(mutex_);
    if (state == SPAS_RUN && state_ != SPAS_RUN) {
      started_ = GetTickCount64();
      base_ = position_;
    }
    state_ = state;
    SetEvent(event_);
    return S_OK;
  }
  STDMETHODIMP SetFormat(REFGUID, const WAVEFORMATEX*) override { return S_OK; }
  STDMETHODIMP GetStatus(SPAUDIOSTATUS* status) override {
    if (!status) return E_POINTER;
    std::lock_guard<std::mutex> lock(mutex_);
    memset(status, 0, sizeof *status);
    status->State = state_;
    const uint64_t due = DueLocked();
    status->cbNonBlockingIO = state_ == SPAS_RUN && due > position_ ? static_cast<long>(due - position_) : 0;
    status->CurSeekPos = position_;
    status->CurDevicePos = position_;
    return S_OK;
  }
  STDMETHODIMP SetBufferInfo(const SPAUDIOBUFFERINFO* info) override {
    if (info) info_ = *info;
    return S_OK;
  }
  STDMETHODIMP GetBufferInfo(SPAUDIOBUFFERINFO* info) override {
    if (info) *info = info_;
    return S_OK;
  }
  STDMETHODIMP GetDefaultFormat(GUID* id, WAVEFORMATEX** out) override { return GetFormat(id, out); }
  STDMETHODIMP_(HANDLE) EventHandle() override { return event_; }
  STDMETHODIMP GetVolumeLevel(ULONG* level) override {
    if (level) *level = 100;
    return S_OK;
  }
  STDMETHODIMP SetVolumeLevel(ULONG) override { return S_OK; }
  STDMETHODIMP GetBufferNotifySize(ULONG* size) override {
    if (size) *size = 0;
    return S_OK;
  }
  STDMETHODIMP SetBufferNotifySize(ULONG) override { return S_OK; }

 private:
  PacedAudio() {
    info_.ulMsMinNotification = 50;
    info_.ulMsBufferSize = 1000;
    info_.ulMsEventBias = 0;
  }
  ~PacedAudio() {
    if (event_) CloseHandle(event_);
  }

  uint64_t DueLocked() const {
    const uint64_t elapsed = GetTickCount64() - started_;
    return base_ + elapsed * format_.nAvgBytesPerSec / 1000 / format_.nBlockAlign * format_.nBlockAlign;
  }

  std::atomic<ULONG> refs_{1};
  std::mutex mutex_;
  WAVEFORMATEX format_{};
  std::vector<uint8_t> data_;
  SPAUDIOSTATE state_ = SPAS_CLOSED;
  SPAUDIOBUFFERINFO info_{};
  uint64_t position_ = 0, base_ = 0, started_ = 0;
  HANDLE event_ = nullptr;
};

}  // namespace sapi_dictation
