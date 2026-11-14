import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import 'aos_support.dart';
import 'overlays.dart';
import 'profile.dart';
import 'skill_copies.dart';

/// Named overlays from git, under [root]: each one's files in `<root>/<name>/`, its record in `<root>/<name>.json`, and
/// the fetched history in `<root>/.cache/<name>.git`. Nothing from a source is ever run: files are read out of git's
/// object store, never checked out, so no hook, filter or attribute of the source applies.
final class OverlayStore implements OverlayManager {
  OverlayStore(this.root, {this.git = 'git', this.maxFiles = defaultMaxFiles, this.maxBytes = defaultMaxBytes,
      this.maxEntries = defaultMaxEntries, this.maxFetchBytes = defaultMaxFetchBytes, this.timeout = defaultTimeout,
      this.skillGitConfig = const [], this.skillProtocols = defaultSkillProtocols})
      : _isolated = false;

  OverlayStore._isolated(this.root, this.git, this.maxFiles, this.maxBytes, this.maxEntries, this.maxFetchBytes,
      this.timeout, this.skillGitConfig, this.skillProtocols)
      : _isolated = true;

  /// The transports a skill may be fetched over.
  static const defaultSkillProtocols = ['https', 'http', 'ssh', 'git'];

  /// The most files one overlay may hold.
  static const defaultMaxFiles = 4096;

  /// The most bytes its files may hold in all.
  static const defaultMaxBytes = 64 * 1024 * 1024;

  /// The most entries a source's tree may list, excluded ones included.
  static const defaultMaxEntries = 16384;

  /// The most bytes one fetch may write into the cache before it is ended.
  static const defaultMaxFetchBytes = 256 * 1024 * 1024;

  /// How long one git command may run before it is ended.
  static const defaultTimeout = Duration(minutes: 5);

  final String root;

  /// The git program: a path, or a name looked for in the absolute directories of `PATH`.
  final String git;
  final int maxFiles, maxBytes, maxEntries, maxFetchBytes;
  final Duration timeout;

  /// Git settings (`key=value`) added to every command that fetches a skill, and the transports allowed for it. Tests map
  /// a made-up host to a local repository with `url.<path>.insteadOf` and allow `file`; nothing else has a use for them.
  final List<String> skillGitConfig;
  final List<String> skillProtocols;

  /// Whether this instance fetches skills: git then runs with an environment of its own, none of the person's
  /// configuration, credentials, agent or prompts, and follows no redirect.
  final bool _isolated;

  /// The instance that fetches skills, held to the limits of one skill's directory.
  late final OverlayStore _skillGit = OverlayStore._isolated(root, git, maxSkillFileCount, maxSkillByteCount,
      maxEntries, maxFetchBytes, timeout, skillGitConfig, skillProtocols);

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
      throw OverlayException('Its tree digest is $digest, not the pin $pin canoa.cfg names: nothing was installed');
    }
    final before = installed(name);
    final old = before?.files ?? const {};
    final manifest = files['OVERLAY.md'] ?? files['overlay.cfg'];
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
  Future<InstalledOverlay> apply(OverlayUpdate update, {String? catalog}) async {
    final name = update.name;
    _checkName(name);
    final before = installed(name);
    final label = catalog ?? (before?.source == update.source.spec ? before?.catalog : null);
    final cache = _cache(name);
    final staging = Directory(_path('.staging-$name'));
    if (staging.existsSync()) await staging.delete(recursive: true);
    await staging.create(recursive: true);
    try {
      final blobs = await _blobs(cache, update.files.values.toSet().toList());
      final written = <String, List<int>>{};
      for (final MapEntry(key: path, value: blob) in update.files.entries) {
        final bytes = blobs[blob]!;
        final file = File('${staging.path}${Platform.pathSeparator}${path.replaceAll('/', Platform.pathSeparator)}');
        await file.parent.create(recursive: true);
        // Exclusive: a name the file system takes for one already written fails the install rather than replacing it.
        await file.create(exclusive: true);
        await file.writeAsBytes(bytes, flush: true);
        written[path] = bytes;
      }
      // The copy used is the copy hashed.
      if (!sameDigest('${treeOfFiles(written).digest.digest}', update.digest)) {
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
      catalog: label,
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

  @override
  Future<Map<String, List<int>>> read(OverlayUpdate update) async {
    _checkName(update.name);
    final blobs = await _blobs(_cache(update.name), update.files.values.toSet().toList());
    return {for (final MapEntry(key: path, value: blob) in update.files.entries) path: blobs[blob]!};
  }

  @override
  Future<void> discard(String name) async {
    _checkName(name);
    if (installed(name) != null) return;
    final cache = Directory(_cache(name));
    if (cache.existsSync()) await cache.delete(recursive: true);
  }

  String _catalogCache(String label) =>
      '${_path('.catalogs')}${Platform.pathSeparator}$label.git';

  void _checkLabel(String label) {
    if (!isEntryName(label) || !aos.validOverlayName(label)) {
      throw const OverlayException('A catalog label is a valid overlay name of at most 64 letters, digits, - or _');
    }
  }

  @override
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source) async {
    _checkLabel(label);
    final cache = _catalogCache(label);
    if (!Directory(cache).existsSync()) {
      await Directory(cache).create(recursive: true);
      await _git(['init', '--quiet', '--bare', '--template=', cache]);
    }
    return _GitTree(this, cache, await _fetch(cache, source));
  }

  @override
  Future<void> removeCatalog(String label) async {
    _checkLabel(label);
    final cache = Directory(_catalogCache(label));
    if (cache.existsSync()) await cache.delete(recursive: true);
  }

  String get _skillsDirectory => _path(skillsDirectory);

  @override
  Future<Map<String, SkillCopy>> skillCopies() async {
    final directory = Directory(_skillsDirectory);
    if (!directory.existsSync()) return {};
    final out = <String, SkillCopy>{};
    for (final entity in directory.listSync(followLinks: false)) {
      if (entity is! File || !entity.path.endsWith('.json')) continue;
      final name = entity.uri.pathSegments.last.replaceFirst(RegExp(r'\.json$'), '');
      try {
        final json = jsonDecode(await entity.readAsString()) as Map<String, dynamic>;
        final named = json['name'];
        // A record names a directory under .skills and nothing else: its name is a skill name, the file's own.
        if (named is! String || !validSkillName(named) || skillKey(named) != name || !validSkillName(name)) continue;
        if (json['overlays'] case final List<dynamic> listed
            when listed.any((overlay) => overlay is! String || !aos.validOverlayName(overlay))) {
          continue;
        }
        out[name] = SkillCopy.fromJson(json, _skillFiles(Directory('$_skillsDirectory${Platform.pathSeparator}$name')));
      } on Object {
        continue;
      }
    }
    return out;
  }

  /// A fetched skill's files, by relative path; none when it is not a plain directory within [skillLimits], which the
  /// digest check then refuses.
  Map<String, List<int>> _skillFiles(Directory directory) {
    final files = <String, List<int>>{};
    if (!directory.existsSync()) return files;
    var bytes = 0;
    for (final entity in directory.listSync(recursive: true, followLinks: false)) {
      if (entity is Link) return {};
      if (entity is! File) continue;
      final relative = entity.path.substring(directory.path.length + 1).replaceAll(Platform.pathSeparator, '/');
      final data = entity.readAsBytesSync();
      bytes += data.length;
      if (files.length >= maxSkillFileCount || bytes > maxSkillByteCount) return {};
      files[relative] = data;
    }
    return files;
  }

  @override
  Future<List<(String, Uint8List)>> fetchSkillFiles(aos.Source source) async {
    if (source.kind != 'git') throw const OverlayException('Canoa fetches skills from git only, and this source is not git');
    if (holdsPassword(source.location)) throw const OverlayException('A skill source holds a password, which none may');
    final fetch = _skillGit;
    final cache = '$_skillsDirectory${Platform.pathSeparator}.fetch-${DateTime.now().microsecondsSinceEpoch}.git';
    await Directory(cache).create(recursive: true);
    try {
      await fetch._git(['init', '--quiet', '--bare', '--template=', cache]);
      final commit = await fetch._fetchRef(cache, source.location, source.reference);
      final (files, _, _) = await fetch._list(cache, commit, source.path ?? '');
      final blobs = await fetch._blobs(cache, files.values.toSet().toList());
      return [for (final MapEntry(key: path, value: blob) in files.entries) (path, Uint8List.fromList(blobs[blob]!))];
    } finally {
      await _delete(Directory(cache));
    }
  }

  @override
  Future<void> saveSkill(SkillCopy copy) async {
    if (!validSkillName(copy.name)) throw const OverlayException('That is not a skill name');
    final key = skillKey(copy.name);
    final separator = Platform.pathSeparator;
    final staging = Directory('$_skillsDirectory$separator.staging-$key');
    await _delete(staging);
    await staging.create(recursive: true);
    try {
      for (final MapEntry(key: path, value: bytes) in copy.files.entries) {
        if (!aos.validRelativePath(path)) throw const OverlayException('A skill file has no valid relative path');
        final file = File('${staging.path}$separator${path.replaceAll('/', separator)}');
        await file.parent.create(recursive: true);
        // Exclusive: a name the file system takes for one already written fails the install rather than replacing it.
        await file.create(exclusive: true);
        await file.writeAsBytes(bytes, flush: true);
      }
    } on Object {
      await _delete(staging);
      rethrow;
    }
    final target = Directory('$_skillsDirectory$separator$key');
    final old = Directory('$_skillsDirectory$separator.old-$key');
    await _delete(old);
    if (target.existsSync()) await target.rename(old.path);
    await staging.rename(target.path);
    await _delete(old);
    final record = File('${target.path}.json');
    final next = File('${record.path}.new');
    await next.writeAsString(copy.toText(), flush: true);
    await next.rename(record.path);
  }

  @override
  Future<void> removeSkill(String name) async {
    if (!validSkillName(name)) throw const OverlayException('That is not a skill name');
    final base = '$_skillsDirectory${Platform.pathSeparator}${skillKey(name)}';
    final record = File('$base.json');
    if (record.existsSync()) await record.delete();
    await _delete(Directory(base));
  }

  static Future<void> _delete(Directory directory) async {
    if (directory.existsSync()) await directory.delete(recursive: true);
  }

  void _checkName(String name) {
    if (!isEntryName(name)) throw const OverlayException('An overlay name has 1 to 64 letters, digits, - or _');
  }

  /// Fetches only what [source] names, one commit deep, and returns its commit. A whole commit id is that object and
  /// nothing else: a branch or tag carrying the same name never stands in for it.
  Future<String> _fetch(String cache, GitOverlay source) => _fetchRef(cache, source.repo, source.reference);

  /// [_fetch] for a repository and a ref; no ref is the repository's HEAD.
  Future<String> _fetchRef(String cache, String repo, String? reference) async {
    if (reference == null) {
      await _git(guard: cache, ['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--depth=1',
          '--', repo, 'HEAD']);
      final head = await _git(['--git-dir=$cache', 'rev-parse', '--verify', '--end-of-options', 'FETCH_HEAD^{commit}']);
      return (head as String).trim();
    }
    if (isFullCommitId(reference)) {
      final direct = await _run(guard: cache, ['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules',
          '--depth=1', '--', repo, reference]);
      if (direct.exitCode != 0) {
        // A server that refuses a want by id still serves its branches and tags, one of which may hold the commit.
        await _git(guard: cache, ['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--depth=1',
            '--', repo,
            '+refs/heads/*:refs/canoa/heads/*', '+refs/tags/*:refs/canoa/tags/*']);
      }
      final found = await _run(['--git-dir=$cache', 'rev-parse', '--verify', '--quiet', '--end-of-options',
          '$reference^{commit}']);
      if (found.exitCode != 0 || (found.stdout as String).trim() != reference) {
        throw const OverlayException('The source has no commit of that id');
      }
      return reference;
    }
    final listed = await _git(['ls-remote', '--heads', '--tags', '--', repo, 'refs/heads/$reference',
        'refs/tags/$reference']) as String;
    final names = {for (final line in const LineSplitter().convert(listed)) line.split('\t').last};
    final remote = ['refs/heads/$reference', 'refs/tags/$reference'].where(names.contains).firstOrNull;
    if (remote == null) throw const OverlayException('The source has no branch or tag of that name');
    final local = 'refs/canoa/${remote.substring('refs/'.length)}';
    await _git(guard: cache, ['--git-dir=$cache', 'fetch', '--quiet', '--no-tags', '--no-recurse-submodules', '--depth=1', '--',
        repo, '+$remote:$local']);
    final found = await _git(['--git-dir=$cache', 'rev-parse', '--verify', '--end-of-options', '$local^{commit}']);
    return (found as String).trim();
  }

  /// Every entry under [path] at [commit], read as git lists it and refused as soon as its files pass [maxFiles] or
  /// [maxBytes], then digested as AOS defines: a link, a submodule, a name that is not a relative path or two names
  /// equal without ASCII case refuse the whole overlay. Entries under an excluded name are left out, never installed.
  Future<(OverlayFiles, List<String>, String)> _list(String cache, String commit, String path) async {
    final prefix = path.isEmpty ? <int>[] : utf8.encode('$path/');
    final entries = <(_Listed, String)>[];
    final folded = <String, String>{};
    var count = 0, bytes = 0;
    void take(List<int> line) {
      if (entries.length >= maxEntries) throw OverlayException('The source holds more than $maxEntries entries');
      final tab = line.indexOf(0x09);
      final fields = ascii.decode(line.sublist(0, tab)).split(RegExp(' +'));
      final full = line.sublist(tab + 1);
      if (full.length <= prefix.length || compareBytes(full.sublist(0, prefix.length), prefix) != 0) return;
      final relative = full.sublist(prefix.length);
      final (mode, type, blob) = (fields[0], fields[1], fields[2]);
      final kind = mode == '120000'
          ? _Kind.symlink
          : type == 'blob'
          ? _Kind.file
          : _Kind.special;
      final components = <List<int>>[[]];
      for (final byte in relative) {
        if (byte == 0x2f) {
          components.add([]);
        } else {
          components.last.add(byte);
        }
      }
      final entry = _Listed(components, kind);
      if (!entry.isExcluded && kind != _Kind.file) {
        throw const OverlayException('The source holds a link or a submodule, which no overlay has');
      }
      if (!entry.isExcluded) {
        // One directory on a file system that ignores case: two names it would merge refuse the overlay.
        final names = entry.path.split('/');
        for (var end = 1; end <= names.length; end++) {
          final name = names.take(end).join('/');
          if (folded.putIfAbsent(name.toLowerCase(), () => name) != name) {
            throw const OverlayException('The source holds two names that differ only in case');
          }
        }
      }
      if (kind == _Kind.file && !entry.isExcluded) {
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
        if (entry.kind == _Kind.file && !entry.isExcluded) (entry, blob),
    ];
    if (members.isEmpty) throw const OverlayException('The source has no files at that path and commit');
    final blobs = await _blobs(cache, {for (final (_, blob) in members) blob}.toList());
    final String digest;
    try {
      digest = '${aos.digestEntries([
        for (final (entry, blob) in entries)
          (
            [for (final part in entry.components) Uint8List.fromList(part)],
            switch (entry.kind) {
              _Kind.file => aos.FileEntry(entry.isExcluded ? Uint8List(0) : Uint8List.fromList(blobs[blob]!)),
              _Kind.symlink => const aos.SymlinkEntry(),
              _Kind.special => const aos.SpecialEntry(),
            },
          ),
      ]).digest}';
    } on aos.AosError catch (error) {
      throw OverlayException('The source cannot be an overlay (${error.errorClass}: ${error.subject})');
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

  Future<Object> _git(List<String> args, {bool binary = false, String? guard}) async {
    final result = await _run(args, binary: binary, guard: guard);
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
      for (final scheme in _isolated ? skillProtocols : const ['https', 'http', 'ssh', 'git', 'file'])
        ...['-c', 'protocol.$scheme.allow=always'],
      '-c', 'core.hooksPath=${_path('.no-hooks')}',
      '-c', 'core.fsmonitor=false',
      '-c', 'credential.interactive=never',
      if (_isolated) ...[
        for (final setting in [...isolatedSettings, ...skillGitConfig]) ...['-c', setting],
      ],
      ...args,
    ],
    environment: _isolated
        ? isolatedEnvironment()
        : const {
            'GIT_TERMINAL_PROMPT': '0',
            'GCM_INTERACTIVE': 'never',
            'SSH_ASKPASS_REQUIRE': 'never',
            'GIT_LITERAL_PATHSPECS': '1',
          },
    includeParentEnvironment: !_isolated,
  );

  /// What every git command that fetches a skill is given: no credential helper, no redirect followed, and every object
  /// received checked.
  static const isolatedSettings = ['credential.helper=', 'http.followRedirects=false', 'transfer.fsckObjects=true'];

  /// The ssh command a skill's fetch uses: none of the person's configuration, keys or agent. OpenSSH takes the home
  /// directory from the password database, not from `HOME`, so its own configuration and identity files are replaced by
  /// [empty], a file with nothing in it, and only that identity may be tried.
  static String sshCommand(String empty) => 'ssh -F "$empty" -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none '
      '-o IdentityFile="$empty" -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no';

  /// What a skill's fetch runs git with: not the person's settings, credential helpers, askpass programs or SSH agent,
  /// only what git needs to find its programs and the proxy the network is reached through. Public so a test reads it.
  Map<String, String> isolatedEnvironment() {
    final home = '${_path(skillsDirectory)}${Platform.pathSeparator}.home';
    Directory(home).createSync(recursive: true);
    // An empty file: Git for Windows cannot open the null device as a configuration file.
    final nothing = '$home${Platform.pathSeparator}empty.config';
    File(nothing).writeAsStringSync('');
    final here = Platform.environment;
    return {
      for (final name in const ['PATH', 'SystemRoot', 'windir', 'TEMP', 'TMP', 'TMPDIR', 'LANG', 'HTTP_PROXY',
        'HTTPS_PROXY', 'NO_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'no_proxy', 'all_proxy'])
        name: ?here[name],
      'HOME': home,
      'USERPROFILE': home,
      'XDG_CONFIG_HOME': home,
      'GIT_CONFIG_GLOBAL': nothing,
      'GIT_CONFIG_NOSYSTEM': '1',
      'GIT_TERMINAL_PROMPT': '0',
      'GCM_INTERACTIVE': 'never',
      'SSH_ASKPASS_REQUIRE': 'never',
      'GIT_ALLOW_PROTOCOL': skillProtocols.join(':'),
      'GIT_SSH_COMMAND': sshCommand(nothing),
      'GIT_LITERAL_PATHSPECS': '1',
    };
  }

  /// [args] run to their end, or ended after [timeout]; with [guard], also ended once that directory holds more than
  /// [maxFetchBytes], which then throws.
  Future<ProcessResult> _run(List<String> args, {bool binary = false, String? guard}) async {
    final process = await _start(args);
    final timer = Timer(timeout, process.kill);
    var tooLarge = false;
    final watch = guard == null
        ? null
        : Timer.periodic(const Duration(milliseconds: 500), (_) {
            var size = 0;
            try {
              for (final entity in Directory(guard).listSync(recursive: true, followLinks: false)) {
                if (entity is File) size += entity.lengthSync();
              }
            } on FileSystemException {
              return;
            }
            if (size > maxFetchBytes) {
              tooLarge = true;
              process.kill();
            }
          });
    try {
      final out = process.stdout.fold<List<int>>([], (all, chunk) => all..addAll(chunk));
      final err = process.stderr.drain<void>();
      final code = await process.exitCode;
      final bytes = await out;
      await err;
      if (tooLarge) throw OverlayException('The source sent more than $maxFetchBytes bytes: nothing was installed');
      return ProcessResult(process.pid, code, binary ? bytes : utf8.decode(bytes, allowMalformed: true), '');
    } finally {
      timer.cancel();
      watch?.cancel();
    }
  }
}

/// A fetched catalog repository at one commit. A path is looked up as git lists it, literally, and only a regular file
/// is read: a link, a submodule or a directory is no file.
final class _GitTree implements FetchedTree {
  _GitTree(this._store, this._cache, this.commit);
  final OverlayStore _store;
  final String _cache;
  @override
  final String commit;

  @override
  Future<TreeFile?> file(String path, {required int maxBytes}) async {
    if (!aos.validRelativePath(path)) return null;
    final listed = await _store._run(['--git-dir=$_cache', 'ls-tree', '-l', '-z', '--full-tree', commit, '--', path],
        binary: true);
    if (listed.exitCode != 0) throw const OverlayException('git ls-tree failed');
    final records = [
      for (final record in utf8.decode(listed.stdout as List<int>, allowMalformed: true).split('\u0000'))
        if (record.isNotEmpty) record,
    ];
    if (records.length != 1) return null;
    final tab = records.single.indexOf('\t');
    if (tab < 0 || records.single.substring(tab + 1) != path) return null;
    final fields = records.single.substring(0, tab).split(RegExp(' +'));
    if (fields.length != 4 || fields[1] != 'blob' || (fields[0] != '100644' && fields[0] != '100755')) return null;
    final size = int.tryParse(fields[3]);
    if (size == null) return null;
    if (size > maxBytes) return (size: size, bytes: null);
    final bytes = (await _store._blobs(_cache, [fields[2]]))[fields[2]]!;
    return (size: size, bytes: bytes.length > maxBytes ? null : bytes);
  }

  @override
  Future<List<String>?> directories(String path) async {
    final folder = path.replaceAll(RegExp(r'/+$'), '');
    if (folder.isNotEmpty && !aos.validRelativePath(folder)) return null;
    final listed = await _store._run([
      '--git-dir=$_cache', 'ls-tree', '-z', '--full-tree', commit,
      if (folder.isNotEmpty) ...['--', '$folder/'],
    ], binary: true);
    if (listed.exitCode != 0) throw const OverlayException('git ls-tree failed');
    final prefix = folder.isEmpty ? '' : '$folder/';
    final names = <String>[];
    var any = false;
    for (final record in utf8.decode(listed.stdout as List<int>, allowMalformed: true).split('\u0000')) {
      final tab = record.indexOf('\t');
      if (tab < 0 || !record.substring(tab + 1).startsWith(prefix)) continue;
      any = true;
      final name = record.substring(tab + 1 + prefix.length);
      if (record.substring(0, tab).split(' ').first == '040000' && !name.contains('/')) names.add(name);
    }
    return any ? names : null;
  }

  /// The most characters of paths one `ls-tree` is given: Windows ends a command line at 32767 characters.
  static const _pathBudget = 8000;

  /// Reads the paths with one `ls-tree` per [_pathBudget] characters of them and one `cat-file --batch` for every blob
  /// wanted, where [file] spends two processes on each path.
  @override
  Future<Map<String, TreeFile?>> readFiles(List<String> paths, {required int maxBytes}) async {
    final out = <String, TreeFile?>{for (final path in paths) path: null};
    final found = <String, ({String id, int size})>{};
    final valid = {for (final path in paths) if (aos.validRelativePath(path)) path}.toList();
    var at = 0;
    while (at < valid.length) {
      var length = 0;
      final chunk = <String>[];
      while (at < valid.length && (chunk.isEmpty || length + valid[at].length + 1 <= _pathBudget)) {
        length += valid[at].length + 1;
        chunk.add(valid[at++]);
      }
      final listed = await _store._run(['--git-dir=$_cache', 'ls-tree', '-l', '-z', '--full-tree', commit, '--', ...chunk],
          binary: true);
      if (listed.exitCode != 0) throw const OverlayException('git ls-tree failed');
      final asked = chunk.toSet();
      for (final record in utf8.decode(listed.stdout as List<int>, allowMalformed: true).split('\u0000')) {
        final tab = record.indexOf('\t');
        if (tab < 0 || !asked.contains(record.substring(tab + 1))) continue;
        final fields = record.substring(0, tab).split(RegExp(' +'));
        if (fields.length != 4 || fields[1] != 'blob' || (fields[0] != '100644' && fields[0] != '100755')) continue;
        final size = int.tryParse(fields[3]);
        if (size != null) found[record.substring(tab + 1)] = (id: fields[2], size: size);
      }
    }
    final wanted = {for (final entry in found.values) if (entry.size <= maxBytes) entry.id}.toList();
    final blobs = await _store._blobs(_cache, wanted);
    for (final MapEntry(key: path, value: entry) in found.entries) {
      if (entry.size > maxBytes) {
        out[path] = (size: entry.size, bytes: null);
      } else {
        final bytes = blobs[entry.id]!;
        out[path] = (size: entry.size, bytes: bytes.length > maxBytes ? null : bytes);
      }
    }
    return out;
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

/// How git lists an entry.
enum _Kind { file, symlink, special }

/// One entry of a source's tree: its name as bytes by component, and what it is.
final class _Listed {
  _Listed(this.components, this.kind);

  final List<List<int>> components;
  final _Kind kind;

  String get path => components.map((part) => utf8.decode(part, allowMalformed: true)).join('/');

  /// Under an excluded name, which is no part of an overlay.
  bool get isExcluded =>
      components.any((part) => aos.excludedComponents.contains(utf8.decode(part, allowMalformed: true))) ||
      kind == _Kind.file && components.last.length >= 4 && path.endsWith('.pyc');
}
