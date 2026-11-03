import 'dart:convert';
import 'dart:io';

import 'overlays.dart';
import 'profile.dart';

/// Named overlays from git, under [root]: each one's files in `<root>/<name>/`, its record in `<root>/<name>.json`, and
/// the fetched history in `<root>/.cache/<name>.git`. Nothing from a source is ever run: files are read out of git's
/// object store, never checked out, so no hook, filter or attribute of the source applies.
final class OverlayStore implements OverlayManager {
  OverlayStore(this.root, {this.git = 'git', this.maxFiles = defaultMaxFiles, this.maxBytes = defaultMaxBytes});

  /// The most files one overlay may hold.
  static const defaultMaxFiles = 4096;

  /// The most bytes its files may hold in all.
  static const defaultMaxBytes = 64 * 1024 * 1024;

  final String root;

  /// The git program, found as the operating system finds a command.
  final String git;
  final int maxFiles, maxBytes;

  String _path(String name) => '$root${Platform.pathSeparator}$name';

  /// The directory an installed overlay's files are in.
  @override
  String directoryOf(String name) => _path(name);

  String _cache(String name) => '${_path('.cache')}${Platform.pathSeparator}$name.git';

  /// The overlay installed as [name], or null.
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

  /// Fetches [source] and says what installing it as [name] would change, touching nothing installed.
  @override
  Future<OverlayUpdate> check(String name, GitOverlay given) async {
    _checkName(name);
    final source = OverlaySource.parseGit(given.spec);
    final cache = _cache(name);
    if (!Directory(cache).existsSync()) {
      await Directory(cache).create(recursive: true);
      await _git(['init', '--quiet', '--bare', '--template=', cache]);
    }
    await _git([
      '--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--prune', '--',
      source.repo, '+refs/heads/*:refs/canoa/heads/*', '+refs/tags/*:refs/canoa/tags/*',
    ]);
    final commit = await _resolve(cache, source.reference);
    final (files, skipped) = await _list(cache, commit, source.path);
    final before = installed(name);
    final old = before?.files ?? const {};
    return OverlayUpdate(
      name: name,
      source: source,
      from: before?.commit,
      commit: commit,
      files: files,
      skipped: skipped,
      added: [for (final path in files.keys) if (!old.containsKey(path)) path],
      changed: [for (final MapEntry(:key, :value) in files.entries) if (old[key] case final blob? when blob != value) key],
      removed: [for (final path in old.keys) if (!files.containsKey(path)) path],
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
    for (final MapEntry(key: path, value: blob) in update.files.entries) {
      final bytes = await _git(['--git-dir=$cache', 'cat-file', 'blob', blob], binary: true) as List<int>;
      final file = File('${staging.path}${Platform.pathSeparator}${path.replaceAll('/', Platform.pathSeparator)}');
      await file.parent.create(recursive: true);
      await file.writeAsBytes(bytes, flush: true);
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

  /// Removes what [name] installed and fetched.
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

  /// A branch, then a tag, then a commit id already fetched.
  Future<String> _resolve(String cache, String reference) async {
    final candidates = [
      'refs/canoa/heads/$reference',
      'refs/canoa/tags/$reference',
      if (RegExp(r'^[0-9a-f]{7,64}$').hasMatch(reference)) reference,
    ];
    for (final candidate in candidates) {
      final result = await _run(['--git-dir=$cache', 'rev-parse', '--verify', '--quiet', '--end-of-options',
          '$candidate^{commit}']);
      if (result.exitCode == 0) return (result.stdout as String).trim();
    }
    throw const OverlayException('The source has no branch, tag or commit of that name');
  }

  Future<(OverlayFiles, List<String>)> _list(String cache, String commit, String path) async {
    final listing = await _git(['--git-dir=$cache', 'ls-tree', '-r', '-l', '-z', '--full-tree', commit, '--',
        if (path.isNotEmpty) path]) as String;
    final prefix = path.isEmpty ? '' : '$path/';
    final files = <String, String>{};
    final skipped = <String>[];
    var bytes = 0;
    for (final line in listing.split('\u0000').where((line) => line.isNotEmpty)) {
      final tab = line.indexOf('\t');
      final fields = line.substring(0, tab).split(RegExp(' +'));
      final full = line.substring(tab + 1);
      if (!full.startsWith(prefix)) continue;
      final relative = full.substring(prefix.length);
      final (mode, type, blob) = (fields[0], fields[1], fields[2]);
      if (type != 'blob' || mode == '120000') {
        skipped.add(relative);
        continue;
      }
      if (!_safeRelative(relative)) {
        throw const OverlayException('The source holds a file name this computer cannot hold safely');
      }
      files[relative] = blob;
      bytes += int.tryParse(fields[3]) ?? 0;
      if (files.length > maxFiles) throw OverlayException('The overlay holds more than $maxFiles files');
      if (bytes > maxBytes) throw OverlayException('The overlay holds more than $maxBytes bytes');
    }
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

  /// git with only the transports a source may name, no prompt, and nothing from the source's configuration.
  Future<ProcessResult> _run(List<String> args, {bool binary = false}) => Process.run(
    git,
    [
      '-c', 'protocol.allow=never',
      for (final scheme in const ['https', 'http', 'ssh', 'git', 'file']) ...['-c', 'protocol.$scheme.allow=always'],
      '-c', 'core.hooksPath=${_path('.no-hooks')}',
      '-c', 'core.fsmonitor=false',
      ...args,
    ],
    environment: const {'GIT_TERMINAL_PROMPT': '0', 'GIT_LITERAL_PATHSPECS': '1'},
    stdoutEncoding: binary ? null : utf8,
  );
}

const _reserved = {'CON', 'PRN', 'AUX', 'NUL', 'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
    'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9'};

/// A `/`-separated path that writes below its directory on every platform: no empty, `.` or `..` segment, no `\`, `:`
/// or control character, and no segment Windows takes for a device or trims.
bool _safeRelative(String path) => path.split('/').every((segment) =>
    segment.isNotEmpty && segment != '.' && segment != '..' && !segment.endsWith('.') && !segment.endsWith(' ') &&
    !segment.contains(RegExp(r'[\\:\u0000-\u001f\u007f]')) &&
    !_reserved.contains(segment.split('.').first.toUpperCase()));
