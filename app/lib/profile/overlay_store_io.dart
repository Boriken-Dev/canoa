import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'aos.dart';
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
    final source = OverlaySource.parseGit(given.spec).pinnedTo(given.pin);
    final cache = _cache(name);
    if (!Directory(cache).existsSync()) {
      await Directory(cache).create(recursive: true);
      await _git(['init', '--quiet', '--bare', '--template=', cache]);
    }
    final commit = await _fetch(cache, source);
    final (files, skipped, digest) = await _list(cache, commit, source.path);
    if (source.pin case final pin? when !sameDigest(pin, digest)) {
      throw OverlayException('Its tree digest is $digest, not the pin $pin Canoa.cfg names: nothing was installed');
    }
    final before = installed(name);
    final old = before?.files ?? const {};
    final manifest = files['overlay.cfg'];
    return OverlayUpdate(
      name: name,
      source: source,
      from: before?.commit,
      fromSource: before?.source,
      fromDigest: before?.digest,
      commit: commit,
      digest: digest,
      files: files,
      skipped: skipped,
      added: [for (final path in files.keys) if (!old.containsKey(path)) path],
      changed: [for (final MapEntry(:key, :value) in files.entries) if (old[key] case final blob? when blob != value) key],
      removed: [for (final path in old.keys) if (!files.containsKey(path)) path],
      manifest: manifest == null ? null : utf8.decode((await _blobs(cache, [manifest]))[manifest]!, allowMalformed: true),
    );
  }

  /// Installs exactly what [update] showed: the files at its commit, whatever the ref names now, checked against its tree
  /// digest as they are written. The overlay's directory is replaced whole, so a file edited there is lost; the person's
  /// own additions belong in their own profile.
  @override
  Future<InstalledOverlay> apply(OverlayUpdate update) async {
    final name = update.name;
    _checkName(name);
    final cache = _cache(name);
    final staging = Directory(_path('.staging-$name'));
    if (staging.existsSync()) await staging.delete(recursive: true);
    await staging.create(recursive: true);
    try {
      final blobs = await _blobs(cache, update.files.values.toSet().toList());
      final written = <AosTreeEntry>[];
      for (final MapEntry(key: path, value: blob) in update.files.entries) {
        final bytes = blobs[blob]!;
        final file = File('${staging.path}${Platform.pathSeparator}${path.replaceAll('/', Platform.pathSeparator)}');
        await file.parent.create(recursive: true);
        // Exclusive: a name the file system takes for one already written fails the install rather than replacing it.
        await file.create(exclusive: true);
        await file.writeAsBytes(bytes, flush: true);
        written.add(AosTreeEntry.file(path, bytes));
      }
      // The copy used is the copy hashed.
      if (!sameDigest(digestEntries(written).digest, update.digest)) {
        throw const OverlayException('The files read do not have the digest shown: nothing was installed');
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
      digest: update.digest,
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

  /// Every entry under [path] at [commit], read as git lists it and refused as soon as its files pass [maxFiles] or
  /// [maxBytes], then digested as AOS defines: a link, a submodule, a name that is not a relative path or two names
  /// equal without ASCII case refuse the whole overlay. Entries under an excluded name are left out, never installed.
  Future<(OverlayFiles, List<String>, String)> _list(String cache, String commit, String path) async {
    final prefix = path.isEmpty ? <int>[] : utf8.encode('$path/');
    final entries = <(AosTreeEntry, String)>[];
    var count = 0, bytes = 0;
    void take(List<int> line) {
      final tab = line.indexOf(0x09);
      final fields = ascii.decode(line.sublist(0, tab)).split(RegExp(' +'));
      final full = line.sublist(tab + 1);
      if (full.length <= prefix.length || compareBytes(full.sublist(0, prefix.length), prefix) != 0) return;
      final relative = full.sublist(prefix.length);
      final (mode, type, blob) = (fields[0], fields[1], fields[2]);
      final kind = mode == '120000'
          ? AosEntryKind.symlink
          : type == 'blob'
          ? AosEntryKind.file
          : AosEntryKind.special;
      final components = <List<int>>[[]];
      for (final byte in relative) {
        if (byte == 0x2f) {
          components.add([]);
        } else {
          components.last.add(byte);
        }
      }
      final entry = AosTreeEntry(components, kind);
      if (kind == AosEntryKind.file && !entry.isExcluded) {
        count++;
        bytes += int.tryParse(fields[3]) ?? 0;
        if (count > maxFiles) throw OverlayException('The overlay holds more than $maxFiles files');
        if (bytes > maxBytes) throw OverlayException('The overlay holds more than $maxBytes bytes');
      }
      entries.add((entry, blob));
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
          take(pending);
          pending.clear();
        }
      }
    } on Object {
      lister.kill();
      rethrow;
    }
    if (await lister.exitCode != 0) throw const OverlayException('git ls-tree failed');
    final members = [
      for (final (entry, blob) in entries)
        if (entry.kind == AosEntryKind.file && !entry.isExcluded) (entry, blob),
    ];
    if (members.isEmpty) throw const OverlayException('The source has no files at that path and commit');
    final blobs = await _blobs(cache, {for (final (_, blob) in members) blob}.toList());
    final String digest;
    try {
      digest = digestEntries([
        for (final (entry, blob) in entries)
          AosTreeEntry(entry.components, entry.kind,
              entry.kind == AosEntryKind.file && !entry.isExcluded ? blobs[blob]! : const []),
      ]).digest;
    } on AosException catch (error) {
      throw OverlayException('The source cannot be an overlay (${error.code}: ${error.subject})');
    }
    return (
      {for (final (entry, blob) in members) entry.path: blob},
      [for (final (entry, _) in entries) if (entry.isExcluded) entry.path],
      digest,
    );
  }

  /// The bytes of each blob in [ids], read from git's object store in one `cat-file --batch`, as stored.
  Future<Map<String, List<int>>> _blobs(String cache, List<String> ids) async {
    if (ids.isEmpty) return {};
    final process = await _start(['--git-dir=$cache', 'cat-file', '--batch']);
    unawaited(process.stderr.drain<void>());
    final collected = BytesBuilder(copy: false);
    final reading = process.stdout.timeout(timeout).listen(collected.add).asFuture<void>();
    try {
      for (final id in ids) {
        process.stdin.add(ascii.encode('$id\n'));
      }
      await process.stdin.close();
      await reading;
    } on Object {
      process.kill();
      rethrow;
    }
    if (await process.exitCode != 0) throw const OverlayException('git cat-file failed');
    final data = collected.takeBytes();
    final out = <String, List<int>>{};
    var at = 0;
    for (final id in ids) {
      final end = data.indexOf(0x0a, at);
      if (end < 0) throw const OverlayException('git cat-file answered short');
      final header = ascii.decode(data.sublist(at, end)).split(' ');
      if (header.length != 3 || header[0] != id || header[1] != 'blob') {
        throw const OverlayException('git cat-file did not give a blob it was asked for');
      }
      final size = int.parse(header[2]);
      at = end + 1;
      if (at + size + 1 > data.length) throw const OverlayException('git cat-file answered short');
      out[id] = Uint8List.sublistView(data, at, at + size);
      at += size + 1;
    }
    return out;
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
