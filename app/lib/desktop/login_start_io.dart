import 'dart:io';

import 'login_start.dart';

/// The command that starts this app with no window: the AppImage when one runs it (its mounted copy goes away), else the
/// running executable.
String _startCommand() => Platform.environment['APPIMAGE'] ?? Platform.resolvedExecutable;

/// The registry key Windows reads for what to start at sign-in, under the user's own hive.
const windowsRunKey = r'Software\Microsoft\Windows\CurrentVersion\Run';

LoginStart? systemLoginStart() {
  final environment = Platform.environment;
  if (Platform.isWindows) return RunKeyLoginStart(key: windowsRunKey, command: _startCommand());
  final home = environment['HOME'];
  if (Platform.isLinux) {
    final config = environment['XDG_CONFIG_HOME'];
    final base = config != null && config.isNotEmpty ? config : (home == null ? null : '$home/.config');
    return base == null ? null : DesktopEntryLoginStart(folder: '$base/autostart', command: _startCommand());
  }
  if (Platform.isMacOS && home != null) {
    return LaunchAgentLoginStart(folder: '$home/Library/LaunchAgents', command: _startCommand());
  }
  return null;
}

/// A value of the Run key under `HKEY_CURRENT_USER\[key]`. [key] is a parameter so a test works in a scratch key and never in the
/// person's own.
final class RunKeyLoginStart implements LoginStart {
  RunKeyLoginStart({required this.key, required this.command, this.name = 'Canoa'});

  final String key, command, name;

  String get _path => 'HKCU\\$key';

  /// The value written: the program quoted, then the word that keeps its window away.
  String get value => '"$command" $backgroundFlag';

  @override
  Future<bool> isOn() async {
    final read = await Process.run('reg', ['query', _path, '/v', name]);
    return read.exitCode == 0;
  }

  @override
  Future<bool> set(bool on) async {
    if (on) {
      final wrote = await Process.run('reg', ['add', _path, '/v', name, '/t', 'REG_SZ', '/d', value, '/f']);
      return wrote.exitCode == 0;
    }
    // A value that is not there is already removed.
    if (!await isOn()) return true;
    final removed = await Process.run('reg', ['delete', _path, '/v', name, '/f']);
    return removed.exitCode == 0;
  }

  /// The value now registered, or null; for a test to read what was written.
  Future<String?> read() async {
    final read = await Process.run('reg', ['query', _path, '/v', name]);
    if (read.exitCode != 0) return null;
    final line = (read.stdout as String).split(RegExp(r'\r?\n')).firstWhere((l) => l.trim().startsWith(name), orElse: () => '');
    final at = line.indexOf('REG_SZ');
    return at < 0 ? null : line.substring(at + 'REG_SZ'.length).trim();
  }

  /// Removes the scratch key a test made; never called on the Run key.
  Future<void> deleteKey() => Process.run('reg', ['delete', _path, '/f']);
}

/// A file in a folder the session reads at sign-in, which [folder] names so that a test uses its own.
abstract base class FileLoginStart implements LoginStart {
  FileLoginStart({required this.folder, required this.command});

  final String folder, command;

  String get fileName;
  String render();

  File get file => File('$folder/$fileName');

  @override
  Future<bool> isOn() => file.exists();

  @override
  Future<bool> set(bool on) async {
    try {
      if (on) {
        await Directory(folder).create(recursive: true);
        await file.writeAsString(render());
      } else if (await file.exists()) {
        await file.delete();
      }
      return true;
    } on FileSystemException {
      return false;
    }
  }
}

/// The freedesktop autostart entry, `canoa.desktop`.
final class DesktopEntryLoginStart extends FileLoginStart {
  DesktopEntryLoginStart({required super.folder, required super.command});

  @override
  String get fileName => 'canoa.desktop';

  /// A value of an Exec line: quoted with its own quotes, backslashes, dollar signs and backticks escaped.
  static String quoteExec(String path) =>
      '"${path.replaceAllMapped(RegExp(r'[\\"$`]'), (m) => '\\${m[0]}').replaceAll('%', '%%')}"';

  @override
  String render() => '[Desktop Entry]\nType=Application\nName=Canoa\nExec=${quoteExec(command)} $backgroundFlag\n'
      'Terminal=false\nX-GNOME-Autostart-enabled=true\n';
}

/// A launch agent of the user, which macOS starts at sign-in.
final class LaunchAgentLoginStart extends FileLoginStart {
  LaunchAgentLoginStart({required super.folder, required super.command});

  @override
  String get fileName => 'dev.boriken.canoa.plist';

  static String _xml(String text) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

  @override
  String render() => '<?xml version="1.0" encoding="UTF-8"?>\n'
      '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
      '<plist version="1.0">\n<dict>\n'
      '  <key>Label</key>\n  <string>dev.boriken.canoa</string>\n'
      '  <key>ProgramArguments</key>\n  <array>\n    <string>${_xml(command)}</string>\n    <string>$backgroundFlag</string>\n  </array>\n'
      '  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n';
}
