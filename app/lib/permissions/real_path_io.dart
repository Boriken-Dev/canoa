import 'dart:io';

import 'permission_rules.dart' show RealPath;

/// [path] with the links resolved in the deepest part of it that exists, and what does not exist yet after it as written; null
/// when a link cannot be followed (a dangling one) or nothing of it can be found.
String? resolveExisting(String path) {
  var current = path;
  final rest = <String>[];
  while (true) {
    try {
      if (FileSystemEntity.typeSync(current, followLinks: false) != FileSystemEntityType.notFound) {
        final resolved = File(current).resolveSymbolicLinksSync();
        return rest.isEmpty ? resolved : '$resolved/${rest.reversed.join('/')}';
      }
    } on FileSystemException {
      return null;
    }
    final parent = File(current).parent.path;
    if (parent == current) return null;
    rest.add(current.substring(parent.length).replaceAll(RegExp(r'^[\\/]+'), ''));
    current = parent;
  }
}

const RealPath platformRealPath = resolveExisting;
