/// What Canoa adds to the `coqui_aos` package: digests as text, a tree held in memory, and byte order.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

/// Whether two digests are the same, comparing the hexadecimal digits without regard to case; false when either is not
/// `sha256:` and 64 of them.
bool sameDigest(String a, String b) {
  try {
    return aos.Digest.parse(a).equals(aos.Digest.parse(b));
  } on aos.AosError {
    return false;
  }
}

/// A tree read: its files with their bytes, and its digest.
typedef ReadTree = ({aos.OverlayTree tree, aos.TreeDigest digest});

/// The tree of [files], by `/`-separated path, with its digest. Throws [aos.AosError] for a path that is no member name.
ReadTree treeOfFiles(Map<String, List<int>> files) {
  final tree = aos.OverlayTree.fromBytes([for (final MapEntry(:key, :value) in files.entries) (key, Uint8List.fromList(value))]);
  return (tree: tree, digest: digestOfTree(tree));
}

/// The digest of a tree that holds every file's bytes. Throws [aos.AosError].
aos.TreeDigest digestOfTree(aos.OverlayTree tree) => aos.digestEntries([
  for (final path in tree.paths())
    (path.split('/'), aos.FileEntry(tree.bytes(path) ?? (throw aos.AosError('io', path)))),
]);

/// Settings that bound a read to [maxFiles] files and [maxBytes] bytes in all.
aos.Settings treeSettings(int maxFiles, int maxBytes) =>
    aos.defaultSettings.copyWith({'treeFiles': maxFiles, 'treeBytes': maxBytes});

/// Orders by UTF-8 bytes, which is by code point.
int compareBytes(List<int> a, List<int> b) {
  final shared = a.length < b.length ? a.length : b.length;
  for (var i = 0; i < shared; i++) {
    if (a[i] != b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

/// A mapping's entries by key bytes: mapping order carries no meaning to Canoa, so every order it shows is this one.
List<MapEntry<String, Object?>> sortedEntries(Object? value) => value is Map
    ? ([for (final MapEntry(:key, :value) in value.entries) MapEntry('$key', value)]
      ..sort((a, b) => compareBytes(utf8.encode(a.key), utf8.encode(b.key))))
    : const [];

/// Whether a URL [location] carries a password in its user information (`scheme://user:secret@host`), which a source
/// Canoa stores or shows may not.
bool holdsPassword(String location) {
  final scheme = location.indexOf('://');
  if (scheme < 0) return false;
  final rest = location.substring(scheme + 3);
  final end = rest.indexOf(RegExp(r'[/?#]'));
  final authority = end < 0 ? rest : rest.substring(0, end);
  final at = authority.lastIndexOf('@');
  return at >= 0 && authority.substring(0, at).contains(':');
}
