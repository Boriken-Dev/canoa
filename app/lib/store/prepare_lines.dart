/// What installing an overlay runs besides copying its files, one line each, for the person to read before saying yes: the
/// dependency files installed into an environment of the overlay's own, and its setup. Nothing of it runs from this.
library;

import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

/// The lines for the overlay whose manifest is [manifest] and whose files are [files], on [os]; none when it has nothing to run or
/// its manifest cannot be applied.
List<String> prepareLines(aos.Document manifest, Map<String, List<int>> files, String os) {
  try {
    var tree = aos.OverlayTree.fromPaths(files.keys);
    for (final MapEntry(:key, :value) in files.entries) {
      tree = tree.withBytes(key, Uint8List.fromList(value));
    }
    final facts = aos.Facts(os)
      ..identity = 'overlay'
      ..tree = tree;
    final found = aos.effective(aos.fromPlain(aos.toPlain(manifest)) as aos.Document, facts);
    return [
      for (final dependency in found.dependencies) _dependency(dependency),
      if (found.setup case final setup?) _setup(setup),
      ..._commands(manifest, os),
    ];
  } on aos.AosError {
    return const [];
  }
}

/// The commands the manifest's `scripts` expose on [os]: a path, or a mapping naming the script, its interpreter and its OSes;
/// `false` exposes none.
List<String> _commands(aos.Document manifest, String os) {
  final scripts = manifest['scripts'];
  if (scripts is! Map) return const [];
  final names = [for (final key in scripts.keys) '$key']..sort();
  final lines = <String>[];
  for (final name in names) {
    final value = scripts[name];
    String? path;
    String? interpreter;
    if (value is String) {
      path = value;
    } else if (value is Map && aos.entryOn(value, os)) {
      path = value['path'] is String ? value['path'] as String : null;
      interpreter = value['interpreter'] is String ? value['interpreter'] as String : null;
    }
    if (path == null) continue;
    interpreter ??= switch (path.split('.').last) {
      'py' => 'python',
      'sh' => 'sh',
      'bash' => 'bash',
      'ps1' => 'powershell',
      _ => 'node',
    };
    lines.add("scripts.$name: a command on the agent's PATH that runs $path under $interpreter when it is called; nothing runs now");
  }
  return lines;
}

String _dependency(aos.Dependency dependency) {
  final locked = dependency.lock != null;
  final how = switch ((dependency.runtime, locked)) {
    ('python', true) => 'pip install, hash-checked, no source builds, into an environment of its own',
    ('python', false) => 'pip install, not pinned (what the index serves that day), into an environment of its own',
    (_, true) => 'npm ci into an environment of its own',
    (_, false) => 'npm install, not pinned, into an environment of its own',
  };
  return 'dependencies: ${dependency.file} ($how; fetches from the network)';
}

String _setup(aos.SetupRun setup) {
  final time = setup.timeout == 0 ? 'no time limit' : 'up to ${setup.timeout} s';
  final network = setup.network ? 'says it needs the network' : 'declares no network, which Canoa cannot keep it to';
  return 'setup: ${setup.path} under ${setup.interpreter} ($time; $network), as install, then sync on each update';
}
