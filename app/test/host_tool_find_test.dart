// Where the app finds its host: CANOA_HOST, else `canoa` beside the app, else a `canoa-host` left from before the rename.
import 'dart:io';

import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:flutter_test/flutter_test.dart';

String _exe(String name) => Platform.isWindows ? '$name.exe' : name;

void main() {
  late Directory dir;
  final said = <String>[];

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('canoa-find');
    said.clear();
  });
  tearDown(() => dir.delete(recursive: true));

  HostTool? find({Map<String, String> environment = const {}}) =>
      findHostToolIn(environment: environment, directory: dir.path, log: said.add);

  String named(String name) => '${dir.path}${Platform.pathSeparator}${_exe(name)}';

  test('the canoa beside the app is the host, and nothing is said', () {
    File(named('canoa')).writeAsStringSync('');
    expect(find()?.path, named('canoa'));
    expect(said, isEmpty);
  });

  test('a canoa-host from before the rename is found, and said once as such', () {
    File(named('canoa-host')).writeAsStringSync('');
    expect(find()?.path, named('canoa-host'));
    expect(said, hasLength(1));
    expect(said.single, contains('canoa-host'));
    expect(said.single, contains('canoa'));
  });

  test('canoa wins over a canoa-host beside it', () {
    File(named('canoa')).writeAsStringSync('');
    File(named('canoa-host')).writeAsStringSync('');
    expect(find()?.path, named('canoa'));
    expect(said, isEmpty);
  });

  test('CANOA_HOST names the host before either', () {
    File(named('canoa')).writeAsStringSync('');
    expect(find(environment: {'CANOA_HOST': '/elsewhere/canoa'})?.path, '/elsewhere/canoa');
    expect(said, isEmpty);
  });

  test('neither beside the app is no host', () {
    expect(find(), isNull);
  });

  test('a host speaks as canoa: or, built before the rename, canoa-host:', () {
    expect(hostSaid('canoa: that key is not waiting'), 'that key is not waiting');
    expect(hostSaid('canoa-host: that key is not waiting'), 'that key is not waiting');
    expect(hostSaid('INFO something else'), isNull);
    expect(hostSaid('canoa serve'), isNull);
  });
}
