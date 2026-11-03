// The file a change of Canoa.cfg is checked in: its own for each check, beside Canoa.cfg, and gone afterwards.
import 'dart:io';

import 'package:canoa/desktop/host_tool_io.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('two checks at once each read their own text, and leave nothing behind', () async {
    final dir = await Directory.systemTemp.createTemp('canoa-beside');
    addTearDown(() => dir.delete(recursive: true));
    final config = '${dir.path}${Platform.pathSeparator}Canoa.cfg';
    Future<String> check(String text) => withFileBeside(config, text, (file) async {
      expect(File(file).parent.path, dir.path);
      await Future<void>.delayed(const Duration(milliseconds: 50));
      return File(file).readAsString();
    });
    expect(await Future.wait([check('first'), check('second')]), ['first', 'second']);
    expect(dir.listSync(), isEmpty);
  });

  test('a file already at a name is never written through', () async {
    final dir = await Directory.systemTemp.createTemp('canoa-beside');
    addTearDown(() => dir.delete(recursive: true));
    final config = '${dir.path}${Platform.pathSeparator}Canoa.cfg';
    // Whatever name a check would take is created first, as a stranger in a shared directory could.
    final planted = <File>[];
    for (final name in ['.Canoa.cfg.$pid.check']) {
      planted.add(File('${dir.path}${Platform.pathSeparator}$name')..writeAsStringSync('theirs'));
    }
    await withFileBeside(config, 'mine', (file) async => File(file).readAsString());
    for (final file in planted) {
      expect(file.readAsStringSync(), 'theirs');
    }
  });
}
