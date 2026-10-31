import 'dart:async';
import 'package:canoa/transport/staged_connection.dart';
import 'package:canoa/transport/agent_transport.dart';
import 'package:flutter_test/flutter_test.dart';

final class Source implements AgentConnection {
  final events = StreamController<String>.broadcast(sync: true);
  bool closed = false;
  @override
  AgentTransportKind get kind => AgentTransportKind.webSocket;
  @override
  Stream<String> get messages => events.stream;
  @override
  Future<void> send(String frame) async => events.add(frame);
  @override
  Future<void> close() async {
    closed = true;
    if (!events.isClosed) await events.close();
  }
}

void main() {
  test('no authentication or ACP frames are dropped at handoff', () async {
    final source = Source();
    final channel = StagedAgentConnection(source);
    source.events.add('hello');
    source.events.add('challenge');
    expect(await channel.nextSetupMessage(), 'hello');
    expect(await channel.nextSetupMessage(), 'challenge');
    source.events.add('first ACP frame');
    channel.allowApplicationMessages();
    final received = <String>[];
    final sub = channel.messages.listen(received.add);
    await Future<void>.delayed(Duration.zero);
    source.events.add('second ACP frame');
    await Future<void>.delayed(Duration.zero);
    expect(received, ['first ACP frame', 'second ACP frame']);
    await sub.cancel();
    await channel.close();
    expect(source.closed, isTrue);
  });

  test('no duplicate handshake read or double promotion', () async {
    final source = Source();
    final channel = StagedAgentConnection(source);
    final waiting = channel.nextSetupMessage();
    expect(() => channel.nextSetupMessage(), throwsStateError);
    expect(() => channel.allowApplicationMessages(), throwsStateError);
    source.events.add('hello');
    expect(await waiting, 'hello');
    channel.allowApplicationMessages();
    expect(() => channel.allowApplicationMessages(), throwsStateError);
    expect(() => channel.nextSetupMessage(), throwsStateError);
    await channel.close();
  });

  test('disconnect prevents authentication success', () async {
    final source = Source();
    final channel = StagedAgentConnection(source);
    final waiting = channel.nextSetupMessage();
    final assertion = expectLater(waiting, throwsStateError);
    await source.events.close();
    await assertion;
    expect(() => channel.allowApplicationMessages(), throwsStateError);
    await channel.close();
  });
}
