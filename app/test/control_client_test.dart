// The control channel's client when the host's link ends while a request is still being sent.
@TestOn('vm')
library;

import 'dart:async';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:flutter_test/flutter_test.dart';

/// A link the host ends during a send: the end is heard before the send returns.
final class _EndsDuringSend implements HostLink {
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    unawaited(_out.close());
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }

  @override
  Future<void> close() async {}
}

void main() {
  test('a link that ends while a request is being sent fails that request, and nothing else is thrown', () async {
    final client = ControlClient(_EndsDuringSend());
    await expectLater(client.call('accounts/list'), throwsA(isA<StateError>()));
    // An error nobody was listening for surfaces on a later turn, and fails the test then.
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(client.over, isTrue);
  });
}
