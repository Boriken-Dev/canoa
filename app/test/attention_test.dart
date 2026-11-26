// The host decides which events happen, who is told and what a notice says; the app shows a notice the host pushed, and tells the host
// which conversation the person is looking at. A conversation that needs the person is also marked on its row.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/attention/host_notices.dart';
import 'package:canoa/attention/notifier.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Notifier implements Notifier {
  final shown = <({String title, String body, String sessionId})>[];
  void Function(String)? open;
  void Function(bool)? watcher;

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async => open = onOpen;
  @override
  Future<bool> get available async => true;
  @override
  Future<bool> requestPermission() async => true;
  @override
  void watchAttention(void Function(bool attended) onChange) => watcher = onChange;
  @override
  Future<void> show({required String title, required String body, required String sessionId}) async =>
      shown.add((title: title, body: body, sessionId: sessionId));
}

/// A control link that answers every request with an empty result, keeps what was asked and pushes what a test says.
final class _Link implements HostLink {
  final inbox = StreamController<String>();
  final calls = <({String method, Map<String, Object?> params})>[];

  @override
  Stream<String> get messages => inbox.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, dynamic>;
    if (request['method'] == null) return;
    calls.add((method: '${request['method']}', params: (request['params'] as Map).cast<String, Object?>()));
    inbox.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': <String, Object?>{}}));
  }

  @override
  Future<void> close() async => inbox.close();

  void notice(Map<String, Object?> params) => inbox.add(jsonEncode({'jsonrpc': '2.0', 'method': 'notify/notice', 'params': params}));

  List<Object?> told(String method) => [for (final call in calls) if (call.method == method) call.params];
}

Future<void> _settle() => Future<void>.delayed(const Duration(milliseconds: 30));

void main() {
  group('the host\'s notices', () {
    late _Link link;
    late _Notifier notifier;
    late HostNotices notices;
    final sessions = {'remote-1': (id: 'local-1', title: 'Fix\nthe build')};

    setUp(() async {
      link = _Link();
      notifier = _Notifier();
      notices = HostNotices(
        control: HostControl.dialing(() async => link),
        notifier: notifier,
        language: () => 'es',
        localSession: (remote) => sessions[remote],
      );
      notices.start();
      await _settle();
    });

    tearDown(() async {
      await notices.stop();
      await link.close();
    });

    test('the app asks for notices in its language and the host\'s words are shown as they are', () async {
      expect(link.told('notify/subscribe'), [
        {'language': 'es'},
      ]);
      link.notice({
        'id': 'n1', 'event': 'permission', 'title': 'Fix the build', 'body': 'Claude asks leave to use a tool', 'urgency': 'high',
        'session': {'id': 'remote-1', 'title': 'Fix the build', 'agent': 'Claude'},
      });
      await _settle();
      final shown = notifier.shown.single;
      expect(shown.title, 'Fix the build');
      expect(shown.body, 'Claude asks leave to use a tool');
      expect(shown.sessionId, 'local-1', reason: 'the conversation of this app, so choosing it opens that one');
    });

    test('a conversation the host knew by no title is called what this app calls it, on one line', () async {
      link.notice({
        'id': 'n2', 'event': 'finished', 'title': 'A conversation', 'body': 'Claude finished its turn',
        'session': {'id': 'remote-1', 'title': null, 'agent': 'Claude'},
      });
      await _settle();
      expect(notifier.shown.single.title, 'Fix the build');
    });

    test('a notice about the host, or a session this app does not have, is still shown', () async {
      link.notice({'id': 'n3', 'event': 'host_updated', 'title': 'Canoa', 'body': 'The host was updated 1.2', 'session': null});
      link.notice({
        'id': 'n4', 'event': 'failed', 'title': 'A conversation', 'body': 'x', 'session': {'id': 'elsewhere', 'title': null},
      });
      await _settle();
      expect(notifier.shown.map((each) => each.sessionId), ['', 'elsewhere']);
    });

    test('the conversation the person looks at is reported while the window is focused, and none otherwise', () async {
      notices.selected = 'remote-1';
      await _settle();
      expect(link.told('notify/attended'), isEmpty, reason: 'the window is not focused yet');
      notices.focused = true;
      await _settle();
      expect(link.told('notify/attended').last, {'session': 'remote-1'});
      notices.selected = 'remote-2';
      await _settle();
      expect(link.told('notify/attended').last, {'session': 'remote-2'});
      notices.focused = false;
      await _settle();
      expect(link.told('notify/attended').last, {'session': null});
      final count = link.told('notify/attended').length;
      notices.focused = false;
      await _settle();
      expect(link.told('notify/attended'), hasLength(count), reason: 'a report that says nothing new is not sent');
    });
  });

  test('canoa.cfg holds the host\'s [notifications] table as it found it, and the app has no field of its own for it', () {
    const text = '[notifications]\npermission = ["desktop", "phone"]\nfinished = []\n'
        '[notifications.notifiers.phone]\nprovider = "ntfy"\n';
    final written = CanoaConfig.parse(text).documentToWrite;
    expect(written['notifications'], {
      'permission': ['desktop', 'phone'],
      'finished': <Object?>[],
      'notifiers': {'phone': {'provider': 'ntfy'}},
    });
    expect(CanoaConfig.ownTables, isNot(contains('notifications')));
    expect(CanoaConfig.parse('').documentToWrite.containsKey('notifications'), isFalse, reason: 'a config that names none writes none');
  });
}
