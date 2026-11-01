import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/sealed_channel.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter_test/flutter_test.dart';

final class Wire implements ByteWebChannel {
  final incoming = StreamController<Uint8List>();
  final sent = <Uint8List>[];
  bool closed = false;
  @override
  WebChannelKind get kind => WebChannelKind.webSocket;
  @override
  Stream<String> get messages => incoming.stream.map(decodeText);
  @override
  Stream<Uint8List> get bytes => incoming.stream;
  @override
  Future<void> send(String message) => throw StateError('plain text after a proof');
  @override
  Future<void> sendBytes(Uint8List message) async => sent.add(message);
  @override
  Future<void> close() async => closed = true;
}

Uint8List shifted(Uint8List input) => Uint8List.fromList(input.map((byte) => byte + 1).toList());

void main() {
  test('records go out in call order, however long each took to seal', () async {
    final wire = Wire();
    final slow = Completer<Uint8List>();
    var calls = 0;
    final channel = SealedChannel(
      wire,
      seal: (message) => calls++ == 0 ? slow.future : Future.value(message),
      open: (record) async => record,
      release: () {},
    );
    final first = channel.send('first');
    final second = channel.send('second');
    await Future<void>.delayed(Duration.zero);
    expect(wire.sent, isEmpty);
    slow.complete(utf8.encode('first'));
    await Future.wait([first, second]);
    expect(wire.sent.map(utf8.decode), ['first', 'second']);
  });

  test('a message that does not open ends the channel and releases the session', () async {
    final wire = Wire();
    var released = false;
    final channel = SealedChannel(
      wire,
      seal: (message) async => message,
      open: (record) async => record.first == 0 ? throw StateError('bad record') : shifted(record),
      release: () => released = true,
    );
    final received = channel.messages.toList();
    wire.incoming.add(utf8.encode('`'));
    wire.incoming.add(Uint8List.fromList([0]));
    await expectLater(received, throwsA(isA<TransportProtocolFailure>()));
    await Future<void>.delayed(Duration.zero);
    expect(wire.closed, isTrue);
    expect(released, isTrue);
    await expectLater(channel.send('after'), throwsA(isA<TransportUnavailable>()));
  });
}
