import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'overlays.dart';
import 'profile.dart';

/// Named overlays from git, under [root]: each one's files in `<root>/<name>/`, its record in `<root>/<name>.json`, and
/// the fetched history in `<root>/.cache/<name>.git`. Nothing from a source is ever run: files are read out of git's
/// object store, never checked out, so no hook, filter or attribute of the source applies.
final class OverlayStore implements OverlayManager {
  OverlayStore(this.root, {this.git = 'git', this.maxFiles = defaultMaxFiles, this.maxBytes = defaultMaxBytes,
      this.timeout = defaultTimeout});

  /// The most files one overlay may hold.
  static const defaultMaxFiles = 4096;

  /// The most bytes its files may hold in all.
  static const defaultMaxBytes = 64 * 1024 * 1024;

  /// How long one git command may run before it is ended.
  static const defaultTimeout = Duration(minutes: 5);

  final String root;

  /// The git program: a path, or a name looked for in the absolute directories of `PATH`.
  final String git;
  final int maxFiles, maxBytes;
  final Duration timeout;

  /// [git] as a full path, found once; a name is never looked for in the current directory, which Windows would
  /// otherwise search first.
  late final String gitPath = _find(git);

  String _path(String name) => '$root${Platform.pathSeparator}$name';

  @override
  String directoryOf(String name) => _path(name);

  String _cache(String name) => '${_path('.cache')}${Platform.pathSeparator}$name.git';

  @override
  InstalledOverlay? installed(String name) {
    _checkName(name);
    final record = File('${_path(name)}.json');
    if (!record.existsSync()) return null;
    try {
      return InstalledOverlay.fromJson(name, jsonDecode(record.readAsStringSync()) as Map<String, dynamic>);
    } on Object {
      return null;
    }
  }

  @override
  Future<OverlayUpdate> check(String name, GitOverlay given) async {
    _checkName(name);
    final source = OverlaySource.parseGit(given.spec);
    final cache = _cache(name);
    if (!Directory(cache).existsSync()) {
      await Directory(cache).create(recursive: true);
      await _git(['init', '--quiet', '--bare', '--template=', cache]);
    }
    final commit = await _fetch(cache, source);
    final (files, skipped) = await _list(cache, commit, source.path);
    final before = installed(name);
    final old = before?.files ?? const {};
    final profile = files['profile.toml'];
    return OverlayUpdate(
      name: name,
      source: source,
      from: before?.commit,
      fromSource: before?.source,
      commit: commit,
      files: files,
      skipped: skipped,
      added: [for (final path in files.keys) if (!old.containsKey(path)) path],
      changed: [for (final MapEntry(:key, :value) in files.entries) if (old[key] case final blob? when blob != value) key],
      removed: [for (final path in old.keys) if (!files.containsKey(path)) path],
      profile: profile == null
          ? null
          : utf8.decode(await _git(['--git-dir=$cache', 'cat-file', 'blob', profile], binary: true) as List<int>,
              allowMalformed: true),
    );
  }

  /// Installs exactly what [update] showed: the files at its commit, whatever the ref names now. The overlay's directory
  /// is replaced whole, so a file edited there is lost; the person's own additions belong in their own profile.
  @override
  Future<InstalledOverlay> apply(OverlayUpdate update) async {
    final name = update.name;
    _checkName(name);
    final cache = _cache(name);
    final staging = Directory(_path('.staging-$name'));
    if (staging.existsSync()) await staging.delete(recursive: true);
    await staging.create(recursive: true);
    try {
      for (final MapEntry(key: path, value: blob) in update.files.entries) {
        final bytes = await _git(['--git-dir=$cache', 'cat-file', 'blob', blob], binary: true) as List<int>;
        final file = File('${staging.path}${Platform.pathSeparator}${path.replaceAll('/', Platform.pathSeparator)}');
        await file.parent.create(recursive: true);
        // Exclusive: a name the file system takes for one already written fails the install rather than replacing it.
        await file.create(exclusive: true);
        await file.writeAsBytes(bytes, flush: true);
      }
    } on Object {
      await staging.delete(recursive: true);
      rethrow;
    }
    final target = Directory(_path(name));
    final old = Directory(_path('.old-$name'));
    if (old.existsSync()) await old.delete(recursive: true);
    if (target.existsSync()) await target.rename(old.path);
    await staging.rename(target.path);
    if (old.existsSync()) await old.delete(recursive: true);
    final record = InstalledOverlay(
      name: name,
      source: update.source.spec,
      commit: update.commit,
      files: update.files,
      skipped: update.skipped,
      installed: DateTime.now(),
    );
    final file = File('${_path(name)}.json');
    final next = File('${file.path}.new');
    await next.writeAsString(const JsonEncoder.withIndent('  ').convert(record.toJson()), flush: true);
    await next.rename(file.path);
    return record;
  }

  @override
  Future<void> remove(String name) async {
    _checkName(name);
    for (final path in [_path(name), _cache(name)]) {
      if (Directory(path).existsSync()) await Directory(path).delete(recursive: true);
    }
    final record = File('${_path(name)}.json');
    if (record.existsSync()) await record.delete();
  }

  void _checkName(String name) {
    if (!isEntryName(name)) throw const OverlayException('An overlay name has 1 to 64 letters, digits, - or _');
  }

  /// Fetches only what [source] names, one commit deep, and returns its commit. A whole commit id is that object and
  /// nothing else: a branch or tag carrying the same name never stands in for it.
  Future<String> _fetch(String cache, GitOverlay source) async {
    final reference = source.reference;
    if (isFullCommitId(reference)) {
      final direct = await _run(['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules',
          '--depth=1', '--', source.repo, reference]);
      if (direct.exitCode != 0) {
        // A server that refuses a want by id still serves its branches and tags, one of which may hold the commit.
        await _git(['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--', source.repo,
            '+refs/heads/*:refs/canoa/heads/*', '+refs/tags/*:refs/canoa/tags/*']);
      }
      final found = await _run(['--git-dir=$cache', 'rev-parse', '--verify', '--quiet', '--end-of-options',
          '$reference^{commit}']);
      if (found.exitCode != 0 || (found.stdout as String).trim() != reference) {
        throw const OverlayException('The source has no commit of that id');
      }
      return reference;
    }
    final listed = await _git(['ls-remote', '--heads', '--tags', '--', source.repo, 'refs/heads/$reference',
        'refs/tags/$reference']) as String;
    final names = {for (final line in const LineSplitter().convert(listed)) line.split('\t').last};
    final remote = ['refs/heads/$reference', 'refs/tags/$reference'].where(names.contains).firstOrNull;
    if (remote == null) throw const OverlayException('The source has no branch or tag of that name');
    final local = 'refs/canoa/${remote.substring('refs/'.length)}';
    await _git(['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--depth=1', '--',
        source.repo, '+$remote:$local']);
    final found = await _git(['--git-dir=$cache', 'rev-parse', '--verify', '--end-of-options', '$local^{commit}']);
    return (found as String).trim();
  }

  /// Every file under [path] at [commit], read as git lists it and refused as soon as it passes [maxFiles] or
  /// [maxBytes], before the rest is listed.
  Future<(OverlayFiles, List<String>)> _list(String cache, String commit, String path) async {
    final prefix = path.isEmpty ? '' : '$path/';
    final files = <String, String>{};
    final skipped = <String>[];
    final folded = <String, String>{};
    var bytes = 0;
    void take(String line) {
      final tab = line.indexOf('\t');
      final fields = line.substring(0, tab).split(RegExp(' +'));
      final full = line.substring(tab + 1);
      if (!full.startsWith(prefix)) return;
      final relative = full.substring(prefix.length);
      final (mode, type, blob) = (fields[0], fields[1], fields[2]);
      if (type != 'blob' || mode == '120000') {
        skipped.add(relative);
        return;
      }
      if (!_safeRelative(relative)) {
        throw const OverlayException('The source holds a file name this computer cannot hold safely');
      }
      final segments = relative.split('/');
      for (var end = 1; end <= segments.length; end++) {
        final name = segments.take(end).join('/');
        final other = folded.putIfAbsent(name.toLowerCase(), () => name);
        if (other != name) {
          throw const OverlayException('The source holds two names that differ only in case');
        }
      }
      files[relative] = blob;
      bytes += int.tryParse(fields[3]) ?? 0;
      if (files.length > maxFiles) throw OverlayException('The overlay holds more than $maxFiles files');
      if (bytes > maxBytes) throw OverlayException('The overlay holds more than $maxBytes bytes');
    }
    final lister = await _start(['--git-dir=$cache', 'ls-tree', '-r', '-l', '-z', '--full-tree', commit, '--',
        if (path.isNotEmpty) path]);
    unawaited(lister.stderr.drain<void>());
    final pending = <int>[];
    try {
      await for (final chunk in lister.stdout.timeout(timeout)) {
        for (final byte in chunk) {
          if (byte != 0) {
            pending.add(byte);
            continue;
          }
          take(utf8.decode(pending));
          pending.clear();
        }
      }
    } on Object {
      lister.kill();
      rethrow;
    }
    if (await lister.exitCode != 0) throw const OverlayException('git ls-tree failed');
    if (files.isEmpty) throw const OverlayException('The source has no files at that path and commit');
    return (files, skipped);
  }

  Future<Object> _git(List<String> args, {bool binary = false}) async {
    final result = await _run(args, binary: binary);
    if (result.exitCode != 0) {
      throw OverlayException('git ${args.firstWhere((arg) => !arg.startsWith('-'))} failed (status ${result.exitCode})');
    }
    return result.stdout as Object;
  }

  /// git with only the transports a source may name, nothing asked of the person, and nothing from the source's
  /// configuration; ended after [timeout].
  Future<Process> _start(List<String> args) => Process.start(
    gitPath,
    [
      '-c', 'protocol.allow=never',
      for (final scheme in const ['https', 'http', 'ssh', 'git', 'file']) ...['-c', 'protocol.$scheme.allow=always'],
      '-c', 'core.hooksPath=${_path('.no-hooks')}',
      '-c', 'core.fsmonitor=false',
      '-c', 'credential.interactive=never',
      ...args,
    ],
    environment: const {
      'GIT_TERMINAL_PROMPT': '0',
      'GCM_INTERACTIVE': 'never',
      'SSH_ASKPASS_REQUIRE': 'never',
      'GIT_LITERAL_PATHSPECS': '1',
    },
  );

  Future<ProcessResult> _run(List<String> args, {bool binary = false}) async {
    final process = await _start(args);
    final timer = Timer(timeout, process.kill);
    try {
      final out = process.stdout.fold<List<int>>([], (all, chunk) => all..addAll(chunk));
      final err = process.stderr.drain<void>();
      final code = await process.exitCode;
      final bytes = await out;
      await err;
      return ProcessResult(process.pid, code, binary ? bytes : utf8.decode(bytes, allowMalformed: true), '');
    } finally {
      timer.cancel();
    }
  }
}

/// [program] as a full path: itself when it names a path, else the first match in an absolute directory of `PATH`.
String _find(String program) {
  if (program.contains('/') || program.contains(Platform.pathSeparator)) return program;
  final names = Platform.isWindows ? ['$program.exe', '$program.cmd'] : [program];
  for (final dir in (Platform.environment['PATH'] ?? '').split(Platform.isWindows ? ';' : ':')) {
    if (!isAbsolutePath(dir)) continue;
    for (final name in names) {
      final candidate = '$dir${Platform.pathSeparator}$name';
      if (File(candidate).existsSync()) return candidate;
    }
  }
  throw OverlayException('$program is not in an absolute directory of PATH');
}

const _reserved = {'CON', 'PRN', 'AUX', 'NUL', 'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
    'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9'};

/// A `/`-separated path that writes below its directory on every platform, and that git and Windows read as written:
/// no empty, `.` or `..` segment; no `\`, `:`, `"`, `*`, `<`, `>`, `?`, `|` or control character; no `.git` in any
/// case, no `~` followed by a digit (a Windows short name, such as `git~1`), no character a file system may ignore;
/// and no segment Windows takes for a device or trims.
bool _safeRelative(String path) => path.split('/').every((segment) =>
    segment.isNotEmpty && segment != '.' && segment != '..' && !segment.endsWith('.') && !segment.endsWith(' ') &&
    segment.toLowerCase() != '.git' && !RegExp(r'~[0-9]').hasMatch(segment) &&
    !segment.contains(RegExp('[\\\\:"*<>?|\u0000-\u001f\u007f\u200B-\u200F\u202A-\u202E\u2060-\u206F\uFEFF]')) &&
    !_reserved.contains(segment.split('.').first.toUpperCase()));
