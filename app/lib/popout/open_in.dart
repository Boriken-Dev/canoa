/// Where a plain click on a conversation, and Open on a file or a terminal, shows it (`[appearance] open_in`). Open in new window
/// stays in each menu whichever is chosen.
enum OpenIn {
  /// In the window the click was made in.
  thisWindow('this_window'),

  /// In a window of its own.
  newWindows('new_windows');

  const OpenIn(this.key);

  /// The value in canoa.cfg.
  final String key;

  /// The choice [key] names, or null when it names none.
  static OpenIn? named(String key) => values.where((each) => each.key == key).firstOrNull;
}
