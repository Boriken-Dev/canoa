// The app and the host find the same canoa.cfg files, in the same order: the real host's `canoa layers` against the app's
// ConfigFiles, in a temporary layout with an installed and a system file, for no CANOA_CONFIG, a plain list and a `+` list.
//
// CANOA_TEST_HOST names a built canoa (`cargo build -p canoa-host --bin canoa`); without it the tests are skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/config_layers.dart';
import 'package:flutter_test/flutter_test.dart';

final String? _host = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path => File(path).absolute.path,
  null => null,
};

void main() {
  late Directory root;
  late Map<String, String> layout;

  setUp(() {
    root = Directory.systemTemp.createTempSync('canoa-layers-host');
    for (final dir in ['installed', 'system', 'user', 'home', 'lists']) {
      Directory('${root.path}/$dir').createSync();
    }
    File('${root.path}/installed/canoa.cfg').writeAsStringSync('');
    layout = {
      'CANOA_INSTALLED_DIR': '${root.path}/installed',
      'CANOA_CONFIG': '',
      if (Platform.isWindows) ...{
        'APPDATA': '${root.path}/user',
        'PROGRAMDATA': '${root.path}/system',
        'USERPROFILE': '${root.path}/home',
      } else ...{
        'XDG_CONFIG_HOME': '${root.path}/user',
        'HOME': '${root.path}/home',
      },
    };
  });
  tearDown(() => root.deleteSync(recursive: true));

  String same(String path) => Platform.isWindows ? path.replaceAll('/', r'\').toLowerCase() : path;

  String list(List<String> files) => files.join(Platform.isWindows ? ';' : ':');

  Future<({List<String> files, String own})> hostSays(Map<String, String> env) async {
    final run = await Process.run(_host!, ['layers', '--format', 'json'], environment: env);
    expect(run.exitCode, 0, reason: '${run.stderr}');
    final said = jsonDecode((run.stdout as String).trim()) as Map<String, dynamic>;
    return (files: [for (final file in said['files'] as List) same('$file')], own: same('${said['own']}'));
  }

  ({List<String> files, String own}) appSays(Map<String, String> env) {
    final files = ConfigFiles.resolve(
      env: {...Platform.environment, ...env},
      windows: Platform.isWindows,
      programDir: root.path,
      exists: (path) => File(path).existsSync(),
    );
    return (files: [for (final file in files.files) same(file.path)], own: same(files.own));
  }

  File made(String name) => File('${root.path}/lists/$name')..writeAsStringSync('');

  final skip = _host == null ? 'CANOA_TEST_HOST is not set' : null;

  test('no CANOA_CONFIG: the installed, system and user files', () async {
    final env = {...layout};
    final said = await hostSays(env);
    final app = appSays(env);
    expect(said.files, app.files);
    expect(said.own, app.own);
    expect(said.files, hasLength(3));
    expect(said.files.first, same('${root.path}/installed/canoa.cfg'));
    expect(said.own, same('${root.path}/user/Canoa/canoa.cfg'));
  }, skip: skip);

  test('a plain list: only the listed files, the last written', () async {
    final a = made('a.cfg').path, b = made('b.cfg').path;
    for (final files in [[a], [a, b]]) {
      final env = {...layout, 'CANOA_CONFIG': list(files)};
      final said = await hostSays(env);
      final app = appSays(env);
      expect(said.files, app.files);
      expect(said.own, app.own);
      expect(said.files, [for (final file in files) same(file)]);
      expect(said.own, same(files.last));
    }
  }, skip: skip);

  test('a + list: the listed files over the installed, system and user files', () async {
    final a = made('a.cfg').path, b = made('b.cfg').path;
    for (final files in [[a], [a, b]]) {
      final env = {...layout, 'CANOA_CONFIG': '+${list(files)}'};
      final said = await hostSays(env);
      final app = appSays(env);
      expect(said.files, app.files);
      expect(said.own, app.own);
      expect(said.files, hasLength(3 + files.length));
      expect(said.files.sublist(3), [for (final file in files) same(file)]);
      expect(said.own, same(files.last));
    }
  }, skip: skip);

  test('an empty entry and a missing file are refused by both', () async {
    final a = made('a.cfg').path;
    for (final value in [
      '${list([a])}${Platform.isWindows ? ';' : ':'}',
      '+',
      list([a, '${root.path}/lists/missing.cfg']),
    ]) {
      final env = {...layout, 'CANOA_CONFIG': value};
      final run = await Process.run(_host!, ['layers'], environment: env);
      expect(run.exitCode, isNot(0), reason: value);
      expect(() => appSays(env), throwsFormatException, reason: value);
    }
  }, skip: skip);
}
