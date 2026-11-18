// What installing an overlay runs besides copying its files, as the page lists it before anything is installed: the same lines the
// host writes for its own plan, so what the person reads is what runs.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/store/prepare_lines.dart';
import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter_test/flutter_test.dart';

Map<String, List<int>> _files(Map<String, String> text) => {for (final MapEntry(:key, :value) in text.entries) key: utf8.encode(value)};

List<String> _lines(Map<String, String> text, {String os = 'linux'}) {
  final files = _files(text);
  var tree = aos.OverlayTree.fromPaths(files.keys);
  for (final MapEntry(:key, :value) in files.entries) {
    tree = tree.withBytes(key, Uint8List.fromList(value));
  }
  return prepareLines(aos.readOverlay(tree, aos.defaultSettings).manifest, files, os);
}

void main() {
  test('a requirements file and a setup are listed, in the order they run', () {
    final lines = _lines({
      'overlay.cfg': 'name = "needy"\n',
      'lib/python/requirements.txt': 'requests==2.0\n',
      'setup.py': 'pass\n',
    });
    expect(lines, [
      'dependencies: lib/python/requirements.txt (pip install, not pinned (what the index serves that day), into an environment of its own; fetches from the network)',
      'setup: setup.py under python (up to 300 s; says it needs the network), as install, then sync on each update',
    ]);
  });

  test('a lock is hash-checked, a package.json with a lock is npm ci, and a setup with no limit says so', () {
    final lines = _lines({
      'overlay.cfg': 'name = "needy"\n[[setup]]\ninterpreter = "sh"\npath = "setup.sh"\ntimeout = 0\nnetwork = false\n',
      'lib/python/requirements.txt': 'requests==2.0 --hash=sha256:${'a' * 64}\n',
      'lib/node/package.json': '{"dependencies": {}}\n',
      'lib/node/package-lock.json': '{}\n',
      'setup.sh': 'true\n',
    });
    expect(lines[0], startsWith('dependencies: lib/python/requirements.txt (pip install, hash-checked, no source builds'));
    expect(lines[1], startsWith('dependencies: lib/node/package.json (npm ci into an environment of its own'));
    expect(lines[2], 'setup: setup.sh under sh (no time limit; declares no network, which Canoa cannot keep it to), as install, then sync on each update');
  });

  test('each command the scripts name is listed, as one that runs nothing until it is called', () {
    final lines = _lines({
      'overlay.cfg': 'name = "needy"\n[scripts]\ncheck = "bin/check.py"\nold = false\nlint = { path = "tools/lint.sh", interpreter = "bash", os = ["linux"] }\nwin = { path = "w.ps1", os = ["windows"] }\n',
      'bin/check.py': 'pass\n',
      'tools/lint.sh': 'true\n',
      'w.ps1': 'x\n',
    });
    expect(lines, [
      "scripts.check: a command on the agent's PATH that runs bin/check.py under python when it is called; nothing runs now",
      "scripts.lint: a command on the agent's PATH that runs tools/lint.sh under bash when it is called; nothing runs now",
    ]);
  });

  test('an overlay with nothing to run lists nothing', () {
    expect(_lines({'overlay.cfg': 'name = "plain"\n', 'skills/a/SKILL.md': '---\nname: a\n---\nx\n'}), isEmpty);
  });
}
