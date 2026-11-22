import 'login_start_stub.dart'
    if (dart.library.io) 'login_start_io.dart' as platform;

/// Whether the app starts when this person signs in to the computer. Per user, never for the machine.
abstract interface class LoginStart {
  /// Whether an entry that starts the app is registered now.
  Future<bool> isOn();

  /// Registers the entry, or removes it. Returns null once the computer holds what was asked, else why it does not, in the
  /// system's own words.
  Future<String?> set(bool on);
}

/// The word the registered command takes to start with no window, for the tray to hold.
const backgroundFlag = '--background';

/// This computer's own place for it: the Run key of the user's registry hive on Windows, the user's XDG autostart folder on Linux,
/// a launch agent of the user on macOS. Null where the app has no such thing (a page).
LoginStart? systemLoginStart() => platform.systemLoginStart();
