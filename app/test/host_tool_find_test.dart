// How the app finds its host: the program CANOA_HOST names, for tests, else the library it carries, else none. It no longer
// looks for a program beside itself.
import 'dart:io';

import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter_test/flutter_test.dart';

Never _noLibrary() => throw ArgumentError('Failed to load dynamic library');

void main() {
  final said = <String>[];

  setUp(said.clear);

  HostTool? find({Map<String, String> environment = const {}, CanoaNative Function() open = _noLibrary}) =>
      findHostToolIn(environment: environment, open: open, log: said.add);

  test('CANOA_HOST names a program, and the library is not asked for', () {
    final tool = find(environment: {'CANOA_HOST': '/elsewhere/canoa'});
    expect(tool, isA<ProcessHostTool>());
    expect(tool?.path, '/elsewhere/canoa');
    expect(said, isEmpty);
  });

  test('an empty CANOA_HOST names nothing', () {
    expect(find(environment: {'CANOA_HOST': ''}), isNull);
  });

  test('a canoa beside the app is not run: only CANOA_HOST runs a program', () async {
    final dir = await Directory.systemTemp.createTemp('canoa-find');
    addTearDown(() => dir.delete(recursive: true));
    final name = Platform.isWindows ? 'canoa.exe' : 'canoa';
    File('${dir.path}${Platform.pathSeparator}$name').writeAsStringSync('');
    expect(find(), isNull);
  });

  test('no library is no host, and that is said', () {
    expect(find(), isNull);
    expect(said.single, contains('host library'));
  });

  test('a host speaks as canoa: or, built before the rename, canoa-host:', () {
    expect(hostSaid('canoa: that key is not waiting'), 'that key is not waiting');
    expect(hostSaid('canoa-host: that key is not waiting'), 'that key is not waiting');
    expect(hostSaid('INFO something else'), isNull);
    expect(hostSaid('canoa serve'), isNull);
  });
}
