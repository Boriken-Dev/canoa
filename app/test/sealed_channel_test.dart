import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/client.dart';
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
      closeRecord: () async => Uint8List(1),
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
      closeRecord: () async => Uint8List(1),
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

  SealedChannel closing(Wire wire) => SealedChannel(
    wire,
    seal: (message) async => message,
    open: (record) async => record.isEmpty ? null : record,
    closeRecord: () async => utf8.encode('close'),
    release: () {},
  );

  test('the peer\'s close record is an orderly end', () async {
    final wire = Wire();
    final received = closing(wire).messages.toList();
    wire.incoming.add(utf8.encode('hello'));
    wire.incoming.add(Uint8List(0));
    expect(await received, ['hello']);
  });

  test('a connection cut before the close record is a failure', () async {
    final wire = Wire();
    final received = closing(wire).messages.toList();
    wire.incoming.add(utf8.encode('hello'));
    unawaited(wire.incoming.close());
    await expectLater(received, throwsA(isA<TransportUnavailable>()));
    // The channel closes itself in the background once the failure is out.
    await Future<void>.delayed(const Duration(milliseconds: 10));
    expect(wire.sent.map(utf8.decode), ['close']);
  });

  // A refused Noise key's link ends this way: the initiator finishes before the responder decides, and the link ends while
  // the first request is still being sealed.
  test('a link that ends while a request is being sealed fails that request, and leaves no error unhandled', () async {
    final wire = Wire();
    final staged = StagedChannel(wire);
    final sealing = Completer<Uint8List>();
    final channel = SealedChannel(
      staged,
      seal: (message) => sealing.future,
      open: (record) async => record,
      closeRecord: () async => Uint8List(1),
      release: () {},
    );
    final client = AcpClient(
      incoming: channel.messages,
      send: channel.send,
      closeSource: channel.close,
      clientName: 'test',
      clientVersion: '0',
    );
    staged.allowApplicationMessages();
    final initialized = client.initialize();
    await Future<void>.delayed(Duration.zero);
    await wire.incoming.close();
    await Future<void>.delayed(const Duration(milliseconds: 10));
    sealing.complete(Uint8List(1));
    await expectLater(initialized, throwsA(anything));
    await Future<void>.delayed(const Duration(milliseconds: 20));
  });

  test('closing sends the close record after what was queued', () async {
    final wire = Wire();
    final channel = closing(wire);
    await channel.send('last');
    await channel.close();
    expect(wire.sent.map(utf8.decode), ['last', 'close']);
    expect(wire.closed, isTrue);
  });
}
