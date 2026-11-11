import 'dart:ffi';
import 'dart:io';

/// This build can make a session's scratch folder.
const bool hasWorkspaceFolders = true;

String? _variable(Map<String, String> environment, String name, bool windows) {
  if (environment[name] case final value? when value.isNotEmpty) return value;
  if (!windows) return null;
  final upper = name.toUpperCase();
  for (final entry in environment.entries) {
    if (entry.key.toUpperCase() == upper && entry.value.isNotEmpty) return entry.value;
  }
  return null;
}

/// The person's home folder, else null. [environment] and [system] (`windows`, `linux`, `macos`) default to this process's.
String? homeFolder({Map<String, String>? environment, String? system}) {
  final windows = (system ?? Platform.operatingSystem) == 'windows';
  return _variable(environment ?? Platform.environment, windows ? 'USERPROFILE' : 'HOME', windows);
}

/// The person's Documents folder: on Windows the Documents known folder (which follows a redirected or OneDrive Documents;
/// [knownFolder] stands in for asking the system), on Linux `XDG_DOCUMENTS_DIR` from the environment, else from
/// `user-dirs.dirs`, else `~/Documents`, on macOS `~/Documents`. Null when there is no home to put it in.
String? documentsFolder({Map<String, String>? environment, String? system, String? Function()? knownFolder}) {
  final env = environment ?? Platform.environment;
  final os = system ?? Platform.operatingSystem;
  final home = homeFolder(environment: env, system: os);
  if (os == 'windows') {
    final known = (knownFolder ?? _windowsDocuments)();
    if (known != null && known.isNotEmpty) return known;
    return home == null ? null : '$home\\Documents';
  }
  if (os == 'linux') {
    final named = _variable(env, 'XDG_DOCUMENTS_DIR', false) ?? _userDirsDocuments(env, home);
    if (named != null) return _withHome(named, home);
  }
  return home == null ? null : '$home/Documents';
}

String _withHome(String path, String? home) =>
    home != null && (path == r'$HOME' || path.startsWith(r'$HOME/')) ? '$home${path.substring(5)}' : path;

/// `XDG_DOCUMENTS_DIR` as `user-dirs.dirs` writes it, in `$XDG_CONFIG_HOME` or `~/.config`.
String? _userDirsDocuments(Map<String, String> env, String? home) {
  final config = _variable(env, 'XDG_CONFIG_HOME', false) ?? (home == null ? null : '$home/.config');
  if (config == null) return null;
  try {
    for (final line in File('$config/user-dirs.dirs').readAsLinesSync()) {
      final match = RegExp(r'^\s*XDG_DOCUMENTS_DIR\s*=\s*"?([^"]*)"?\s*$').firstMatch(line);
      if (match != null && match.group(1)!.isNotEmpty) return match.group(1);
    }
  } on FileSystemException {
    return null;
  }
  return null;
}

/// FOLDERID_Documents, which `SHGetKnownFolderPath` answers with the folder Windows really uses.
String? _windowsDocuments() {
  if (!Platform.isWindows) return null;
  try {
    final ole32 = DynamicLibrary.open('ole32.dll');
    final shell32 = DynamicLibrary.open('shell32.dll');
    final alloc = ole32.lookupFunction<Pointer<Uint8> Function(IntPtr), Pointer<Uint8> Function(int)>('CoTaskMemAlloc');
    final free = ole32.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('CoTaskMemFree');
    final known = shell32.lookupFunction<
        Int32 Function(Pointer<Uint8>, Uint32, IntPtr, Pointer<Pointer<Uint16>>),
        int Function(Pointer<Uint8>, int, int, Pointer<Pointer<Uint16>>)>('SHGetKnownFolderPath');
    // {FDD39AD0-238F-46AF-ADB4-6C85480369C7}
    const guid = [0xD0, 0x9A, 0xD3, 0xFD, 0x8F, 0x23, 0xAF, 0x46, 0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7];
    final id = alloc(16);
    final out = alloc(sizeOf<IntPtr>()).cast<Pointer<Uint16>>();
    try {
      for (var i = 0; i < 16; i++) {
        id[i] = guid[i];
      }
      out.value = nullptr;
      final result = known(id, 0, 0, out);
      final path = out.value;
      if (result != 0 || path == nullptr) return null;
      try {
        var length = 0;
        while (path[length] != 0) {
          length++;
        }
        return String.fromCharCodes(path.asTypedList(length));
      } finally {
        free(path.cast());
      }
    } finally {
      free(id.cast());
      free(out.cast());
    }
  } on Object {
    return null;
  }
}

/// This computer's name.
String? localHostName() {
  try {
    return Platform.localHostname;
  } on Object {
    return null;
  }
}

String get pathSeparator => Platform.pathSeparator;

/// Makes [path] and what is above it; true when it was not there. One small synchronous call, so a session starting waits on
/// no other disk work first.
Future<bool> makeFolder(String path) async {
  final folder = Directory(path);
  if (folder.existsSync()) return false;
  folder.createSync(recursive: true);
  return true;
}

/// Removes [path] when it is empty, and says nothing when it is not or cannot be.
Future<void> removeEmptyFolder(String path) async {
  try {
    Directory(path).deleteSync();
  } on FileSystemException {
    // Not empty, or already gone: leave it.
  }
}
