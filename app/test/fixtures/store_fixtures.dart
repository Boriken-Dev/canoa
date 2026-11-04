// What the store's tests share: repositories in a temporary directory, real PNG bytes, and an in-memory overlay manager
// for widget tests, which cannot wait on processes.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/profile/aos.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home.dart';
import 'package:canoa/profile/project_trust.dart';
import 'package:flutter_test/flutter_test.dart';

final hasGit = () {
  try {
    return Process.runSync('git', ['--version']).exitCode == 0;
  } on ProcessException {
    return false;
  }
}();

String git(String dir, List<String> args) {
  final result = Process.runSync('git', [
    '-c', 'user.name=Canoa Test', '-c', 'user.email=test@canoa.invalid', '-c', 'commit.gpgsign=false',
    '-c', 'core.autocrlf=false', '-c', 'init.defaultBranch=main', ...args,
  ], workingDirectory: dir);
  expect(result.exitCode, 0, reason: 'git ${args.join(' ')}: ${result.stderr}');
  return (result.stdout as String).trim();
}

/// A working repository and the bare one it pushes to: a catalog's or an overlay's source.
final class Repo {
  Repo(String root, String name) : work = Directory('$root/$name-work'), bare = '$root/$name.git' {
    work.createSync(recursive: true);
    git(work.path, ['init', '--quiet']);
    git(root, ['init', '--quiet', '--bare', bare]);
  }
  final Directory work;
  final String bare;

  /// The bare repository as a source location git takes, with `/` separators.
  String get location => bare.replaceAll(r'\', '/');

  void write(String path, Object content) {
    final file = File('${work.path}/$path');
    file.parent.createSync(recursive: true);
    if (content is String) {
      file.writeAsStringSync(content);
    } else {
      file.writeAsBytesSync(content as List<int>);
    }
  }

  void delete(String path) => File('${work.path}/$path').deleteSync();

  /// Commits everything and pushes it to `main`; the commit's id.
  String commit(String message) {
    git(work.path, ['add', '-A']);
    git(work.path, ['commit', '--quiet', '--allow-empty', '-m', message]);
    git(work.path, ['push', '--quiet', '--force', bare, 'HEAD:refs/heads/main']);
    return git(work.path, ['rev-parse', 'HEAD']);
  }
}

int _crc(List<int> bytes) {
  var crc = 0xffffffff;
  for (final byte in bytes) {
    crc ^= byte;
    for (var bit = 0; bit < 8; bit++) {
      crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xedb88320 : crc >> 1;
    }
  }
  return crc ^ 0xffffffff;
}

List<int> _chunk(String type, List<int> data) {
  final body = [...ascii.encode(type), ...data];
  final length = ByteData(4)..setUint32(0, data.length);
  final crc = ByteData(4)..setUint32(0, _crc(body));
  return [...length.buffer.asUint8List(), ...body, ...crc.buffer.asUint8List()];
}

/// A real RGBA PNG of [width] by [height]: a disc of [color] on transparency, with a band of [accent] across it.
List<int> png(int width, int height, {int color = 0xff2e7d32, int accent = 0xfffbc02d}) {
  final header = ByteData(13)
    ..setUint32(0, width)
    ..setUint32(4, height)
    ..setUint8(8, 8)
    ..setUint8(9, 6);
  final raw = <int>[];
  for (var y = 0; y < height; y++) {
    raw.add(0);
    for (var x = 0; x < width; x++) {
      final dx = x - width / 2, dy = y - height / 2;
      final inside = dx * dx + dy * dy <= (width / 2.2) * (height / 2.2);
      final band = (y - height / 2).abs() < height / 10;
      final argb = !inside ? 0 : band ? accent : color;
      raw.addAll([(argb >> 16) & 0xff, (argb >> 8) & 0xff, argb & 0xff, (argb >> 24) & 0xff]);
    }
  }
  return [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    ..._chunk('IHDR', header.buffer.asUint8List()),
    ..._chunk('IDAT', ZLibCodec().encode(raw)),
    ..._chunk('IEND', const []),
  ];
}

/// A PNG header claiming [width] by [height], with no pixels: refused from the header before anything decodes it.
List<int> pngHeaderOnly(int width, int height) {
  final header = ByteData(13)
    ..setUint32(0, width)
    ..setUint32(4, height)
    ..setUint8(8, 8)
    ..setUint8(9, 6);
  return [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, ..._chunk('IHDR', header.buffer.asUint8List())];
}

/// An SVG with a script, which no name makes an icon.
const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><script>alert(1)</script></svg>\n';

/// Files in memory by source: what a widget test's catalogs and overlays hold.
final class MemoryTree implements FetchedTree {
  MemoryTree(this.files, {this.commit = 'c0ffee0000000000000000000000000000000000'});
  final Map<String, List<int>> files;
  @override
  final String commit;
  @override
  Future<TreeFile?> file(String path, {required int maxBytes}) async {
    final bytes = files[path];
    if (bytes == null) return null;
    return (size: bytes.length, bytes: bytes.length > maxBytes ? null : bytes);
  }
}

/// Overlays and catalogs in memory, keyed by the repository and path a source names: checks, installs and removals
/// as [OverlayManager] defines them, with no git.
final class MemoryOverlays implements OverlayManager {
  /// Repository location to its files, path to bytes.
  final repos = <String, Map<String, List<int>>>{};
  final records = <String, InstalledOverlay>{};
  final installedFiles = <String, Map<String, List<int>>>{};
  final fetched = <String>[];
  final _blobs = <String, List<int>>{};

  void put(String repo, Map<String, Object> files) => repos[repo] = {
    for (final MapEntry(:key, :value) in files.entries) key: value is String ? utf8.encode(value) : value as List<int>,
  };

  @override
  InstalledOverlay? installed(String name) => records[name];

  @override
  String directoryOf(String name) => '/home/overlays/$name';

  @override
  Future<OverlayUpdate> check(String name, GitOverlay source) async {
    fetched.add(source.spec);
    final all = repos[source.repo] ?? (throw const OverlayException('The source has no branch or tag of that name'));
    final prefix = source.path.isEmpty ? '' : '${source.path}/';
    final files = {
      for (final MapEntry(:key, :value) in all.entries)
        if (key.startsWith(prefix)) key.substring(prefix.length): value,
    };
    final digest = digestEntries([for (final MapEntry(:key, :value) in files.entries) AosTreeEntry.file(key, value)]).digest;
    if (source.pin case final pin? when !sameDigest(pin, digest)) throw const OverlayException('pin');
    final blobs = <String, String>{};
    for (final MapEntry(:key, :value) in files.entries) {
      final id = sha256Digest(value);
      _blobs[id] = value;
      blobs[key] = id;
    }
    final before = records[name];
    final old = before?.files ?? const {};
    return OverlayUpdate(
      name: name, source: source, from: before?.commit, fromSource: before?.source, fromDigest: before?.digest,
      commit: digest.substring(7, 47), digest: digest, files: blobs, skipped: const [],
      added: [for (final path in blobs.keys) if (!old.containsKey(path)) path],
      changed: [for (final MapEntry(:key, :value) in blobs.entries) if (old[key] case final blob? when blob != value) key],
      removed: [for (final path in old.keys) if (!blobs.containsKey(path)) path],
      manifest: files['overlay.cfg'] == null ? null : utf8.decode(files['overlay.cfg']!),
    );
  }

  @override
  Future<Map<String, List<int>>> read(OverlayUpdate update) async =>
      {for (final MapEntry(:key, :value) in update.files.entries) key: _blobs[value]!};

  @override
  Future<InstalledOverlay> apply(OverlayUpdate update) async {
    installedFiles[update.name] = await read(update);
    return records[update.name] = InstalledOverlay(name: update.name, source: update.source.spec, commit: update.commit,
        digest: update.digest, files: update.files);
  }

  @override
  Future<void> remove(String name) async {
    records.remove(name);
    installedFiles.remove(name);
  }

  @override
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source) async {
    fetched.add(source.spec);
    return MemoryTree(repos[source.repo] ?? (throw const OverlayException('The source has no branch or tag of that name')));
  }

  @override
  Future<void> removeCatalog(String label) async {}
}

/// A profile home over [MemoryOverlays], reading installed overlays from it.
final class MemoryHome implements ProfileHome {
  MemoryHome(this.overlays);
  @override
  final MemoryOverlays overlays;
  @override
  late final trust = ProjectTrust(read: () async => null, write: (_) async {});
  @override
  String get configDir => '/home';
  @override
  String get os => 'linux';
  @override
  String join(String dir, String relative) => '$dir/$relative';
  @override
  String? canonicalDirectory(String directory) => directory;
  @override
  Future<List<int>?> readBytes(String path) async => null;
  @override
  Future<String?> readSecret(String path, {String? within}) async => null;
  @override
  Future<List<AosTreeEntry>> readTree(String directory, {int maxFiles = 4096, int maxBytes = 64 << 20}) async {
    final name = directory.split('/').last;
    return [
      for (final MapEntry(:key, :value) in (overlays.installedFiles[name] ?? const <String, List<int>>{}).entries)
        AosTreeEntry.file(key, value),
    ];
  }
}
