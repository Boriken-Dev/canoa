// Bringing in earlier sessions through the real host library: a Claude and a Codex session are found in fixture folders, brought
// in, listed as Earlier conversations and read back, a malformed one is refused with what did not carry, and the fixture
// folders are the same bytes afterwards. The host looks in the folders the programs themselves use, so it is pointed at the
// fixtures, never at the person's own:
//
//   CANOA_TEST_LIBRARY=<canoa_ffi> CANOA_TEST_HOST=<canoa> \
//     CLAUDE_CONFIG_DIR=test/fixtures/import_homes/projects_home CODEX_HOME=test/fixtures/import_homes/sessions_home \
//     flutter test test/session_import_host_test.dart
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/history/session_import.dart';
import 'package:canoa/history/session_move.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart';
import 'fixtures/examples.dart';

const _claudeId = '6f2c1b0e-0000-4000-8000-00000000c0de';
const _junkId = '7a7a7a7a-0000-4000-8000-000000000001';
const _codexId = '01a1cb7b-a5cd-7ec1-b69a-c112db39d6d3';

String _fixture(String name) => '${Directory.current.path}/test/fixtures/import_homes/$name'.replaceAll(r'\', '/');

String? _unpointed() {
  for (final (variable, name) in [('CLAUDE_CONFIG_DIR', 'projects_home'), ('CODEX_HOME', 'sessions_home')]) {
    final set = (Platform.environment[variable] ?? '').replaceAll(r'\', '/');
    final here = Directory(set).absolute.path.replaceAll(r'\', '/');
    if (set.isEmpty || !here.endsWith('test/fixtures/import_homes/$name')) {
      return '$variable names test/fixtures/import_homes/$name, so the person\'s own folders are not read';
    }
  }
  return null;
}

/// Every file under the fixture folders with its bytes.
Map<String, List<int>> _bytes() => {
  for (final name in ['projects_home', 'sessions_home'])
    for (final file in Directory(_fixture(name)).listSync(recursive: true).whereType<File>())
      file.path: file.readAsBytesSync(),
};

void main() {
  test('a Claude and a Codex session are brought in and listed, a malformed one is refused, and the folders are unchanged', () async {
    final before = _bytes();
    expect(before, hasLength(3));
    final scratch = Directory.systemTemp.createTempSync('canoa-import-');
    addTearDown(() => scratch.deleteSync(recursive: true));
    String standin(String name) {
      final path = '${scratch.path}/$name${Platform.isWindows ? '.exe' : ''}';
      File(exampleBinary('harness_standin')!).copySync(path);
      return path.replaceAll(r'\', '/');
    }

    final host = await EmbeddedHost.start(
      dir: Directory('${scratch.path}/host')..createSync(),
      config: '[host.agents.Claude]\nharness = "claude"\nexecutable = "${standin('claude')}"\nlisten = "127.0.0.1:0"\n'
          '[host.agents.Codex]\nharness = "codex"\nexecutable = "${standin('codex')}"\nlisten = "127.0.0.1:0"\n',
    );
    addTearDown(host.stop);
    final control = await HostControl(host.supervisor).ready();
    const sources = <ImportSource>[
      (entry: 'Claude', harness: 'claude', label: 'Claude'),
      (entry: 'Codex', harness: 'codex', label: 'Codex'),
    ];

    final found = await listImportable(control, sources);
    expect({for (final each in found) each.id}, {_claudeId, _junkId, _codexId});
    expect(found.firstWhere((each) => each.id == _claudeId).title, 'Fix the failing build');
    expect(found.firstWhere((each) => each.id == _codexId).title, 'Add a health endpoint');
    expect(await importedSessions(control, 'Claude'), isEmpty, reason: 'listing brings nothing in');

    final done = await importChosen(control, found);
    expect(done.imported, 2);
    expect(done.notes, hasLength(1));
    expect(done.notes.single, contains('not imported'));
    expect(done.notes.single, contains('holds no conversation'));

    for (final (entry, id, said) in [('Claude', _claudeId, 'Fixed it.'), ('Codex', _codexId, 'Added /health.')]) {
      final earlier = await importedSessions(control, entry);
      expect([for (final each in earlier) each.id], [id]);
      final text = [for (final each in historyMessages(await sessionHistory(control, entry: entry, session: id))) each.text];
      expect(text, contains(said));
    }
    expect((await listImportable(control, sources)).where((each) => each.imported).map((each) => each.id), unorderedEquals([_claudeId, _codexId]));

    final after = _bytes();
    expect(after.keys, before.keys);
    for (final path in before.keys) {
      expect(after[path], before[path], reason: '$path is only ever read');
    }
  }, skip: embeddedHostMissing() ?? exampleMissing('harness_standin') ?? _unpointed(), timeout: const Timeout(Duration(minutes: 3)));
}
