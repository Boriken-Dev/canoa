// The configuration file's name: canoa.cfg, with a Canoa.cfg from before the rename taken over where names are case sensitive.
import 'dart:io';

import 'package:canoa/config/canoa_config_io.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  late Directory dir;
  final said = <String>[];

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('canoa-cfg');
    said.clear();
  });
  tearDown(() => dir.delete(recursive: true));

  String at(String name) => '${dir.path}${Platform.pathSeparator}$name';
  List<String> names() => dir.listSync().map((entry) => entry.uri.pathSegments.where((s) => s.isNotEmpty).last).toList();

  test('the default place is canoa.cfg in the Canoa folder', () {
    final path = desktopConfigPath();
    if (path != null && Platform.environment['CANOA_CONFIG'] == null) {
      expect(path, endsWith('${Platform.pathSeparator}Canoa${Platform.pathSeparator}canoa.cfg'));
    }
  });

  test('a Canoa.cfg with no canoa.cfg beside it is renamed once, and said', () {
    File(at('Canoa.cfg')).writeAsStringSync('[host]\n');
    expect(adoptLegacyConfigName(at('canoa.cfg'), log: said.add), isTrue);
    expect(File(at('canoa.cfg')).readAsStringSync(), '[host]\n');
    expect(names(), ['canoa.cfg']);
    expect(said, hasLength(1));
    expect(said.single, allOf(contains('Canoa.cfg'), contains('canoa.cfg')));
    expect(adoptLegacyConfigName(at('canoa.cfg'), log: said.add), isFalse);
    expect(said, hasLength(1));
  }, skip: Platform.isWindows || Platform.isMacOS ? 'names ignore case here' : null);

  test('a canoa.cfg that exists is never replaced', () {
    File(at('Canoa.cfg')).writeAsStringSync('old');
    File(at('canoa.cfg')).writeAsStringSync('new');
    expect(adoptLegacyConfigName(at('canoa.cfg'), log: said.add), isFalse);
    expect(File(at('canoa.cfg')).readAsStringSync(), 'new');
    expect(File(at('Canoa.cfg')).readAsStringSync(), 'old');
    expect(said, isEmpty);
  }, skip: Platform.isWindows || Platform.isMacOS ? 'names ignore case here' : null);

  test('where names ignore case, a Canoa.cfg is the canoa.cfg: read as it is, not renamed', () async {
    File(at('Canoa.cfg')).writeAsStringSync('[host]\n');
    expect(adoptLegacyConfigName(at('canoa.cfg'), log: said.add), isFalse);
    expect(File(at('canoa.cfg')).readAsStringSync(), '[host]\n');
    expect(names(), ['Canoa.cfg']);
    expect(said, isEmpty);
  }, skip: Platform.isLinux ? 'names are case sensitive here' : null);
}
