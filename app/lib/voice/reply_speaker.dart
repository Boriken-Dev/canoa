import 'package:flutter/foundation.dart';

/// What the chat's Play buttons and voice mode's switch need of whatever reads replies aloud: the reader of the window that plays the
/// sound, or, in a pop-out window that plays none, the one that asks that window to.
abstract interface class ReplySpeaker implements Listenable {
  /// Whether anything can be read aloud here; false leaves the play buttons and the switch out.
  bool get available;

  /// Whether voice mode is on for the conversation this speaker serves.
  bool get voiceMode;

  /// Whether anything is being read or waits to be.
  bool get busy;

  /// The reply being read, by its key; null when nothing is.
  String? get reading;

  /// Whether the reply [key] is being read or waits to be.
  bool isReading(String key);

  /// Why the last reading failed, in words, and the key of the reply it was for (null for one voice mode was reading).
  String? get error;
  String? get errorKey;
  void clearError();

  /// Reads the reply [key], whose text is [markdown]; reading the one that is being read stops it.
  Future<void> toggle(String key, String markdown);

  /// Ends every reading at once.
  void stop();

  /// Turns voice mode on or off for the conversation this speaker serves.
  void setVoiceMode(bool on);

  /// Gets the audio output ready from inside the press that will lead to speech, which a browser needs.
  void unlock();
}
