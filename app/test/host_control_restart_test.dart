// A request made while the host that was asked to restart is still starting waits for it, as a link through a portal does when the
// host takes longer to answer than one dial waits; a host nobody asked to restart still fails the request at the first dial.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Link implements HostLink {
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': {'ok': true}}));
  }

  @override
  Future<void> close() => _out.close();
}

void main() {
  setUp(() => HostControl.dialPause = const Duration(milliseconds: 20));

  test('a request waits through the dials that fail while the host restarts', () async {
    var dials = 0;
    final control = HostControl.dialing(() async {
      dials++;
      if (dials <= 4) throw TimeoutException('WebSocket could not connect');
      return _Link();
    });
    control.expectRestart(const Duration(seconds: 10));
    final client = await control.ready();
    expect(client.over, isFalse);
    expect(dials, 5);
    await control.close();
  });

  test('with no restart asked, the first dial that fails fails the request', () async {
    var dials = 0;
    final control = HostControl.dialing(() async {
      dials++;
      throw TimeoutException('WebSocket could not connect');
    });
    await expectLater(control.ready(), throwsA(isA<TimeoutException>()));
    expect(dials, 1);
    await control.close();
  });

  test('a host that does not come back fails the request when the wait is over', () async {
    final control = HostControl.dialing(() async => throw TimeoutException('down'));
    control.expectRestart(const Duration(milliseconds: 100));
    await expectLater(control.ready(), throwsA(isA<TimeoutException>()));
    await control.close();
  });
}
