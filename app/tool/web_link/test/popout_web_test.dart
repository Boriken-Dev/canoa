// The web link of a pop-out in a real browser: two ends over a BroadcastChannel carry a conversation, an answer and its end, and a
// window that is not told the name hears nothing.
// Run it with `dart test -p chrome` in this directory (CHROME_EXECUTABLE may name Edge or Chromium); `flutter test --platform chrome` would load the app's
// test configuration, which does not run in a browser.
// ignore_for_file: avoid_relative_lib_imports
@TestOn('browser')
library;

import 'dart:async';

import '../../../lib/popout/platform_web.dart';
import 'package:coqui_acp/coqui_acp.dart' as acp;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/experimental.dart';
import 'package:test/test.dart';

Future<void> _later() => Future<void>.delayed(const Duration(milliseconds: 150));

final class _Quiet extends model.ChatConnection {
  final _updates = StreamController<acp.AcpSessionUpdate>.broadcast();
  @override
  Stream<acp.AcpSessionUpdate> get updates => _updates.stream;
  @override
  Stream<acp.AcpEvent> get events => const Stream.empty();
  @override
  bool get isClosed => false;
  @override
  int? get protocolVersion => 2;
  @override
  Future<String?> prompt(String sessionId, String text, {List<acp.AcpAttachment> attachments = const []}) async => 'm';
  @override
  Future<void> cancel(String sessionId) async {}
  @override
  Future<void> setMode(String sessionId, String modeId) async {}
  @override
  Future<Map<String, dynamic>> setConfigOption(String sessionId, String configId, String value) async => {};
  @override
  Future<void> close() async {}
}

void main() {
  test('a conversation and an answer cross a BroadcastChannel, and the end of the owner is told', () async {
    final session = model.ChatSession(id: 'a', sessionId: 's1');
    final connection = _Quiet();
    session.attach(connection);
    session.apply(acp.AcpSessionUpdate('s1', {
      'sessionUpdate': 'agent_message',
      'messageId': 'm1',
      'content': {'type': 'text', 'text': 'Hello'},
    }));
    final answer = session.askPermission(const acp.AcpPermissionRequest(
      sessionId: 's1',
      toolCallId: 't1',
      title: 'Edit a.txt',
      kind: 'edit',
      options: [
        acp.AcpPermissionOption(optionId: 'allow', name: 'Yes', kind: 'allow_once'),
        acp.AcpPermissionOption(optionId: 'no', name: 'No', kind: 'reject_once'),
      ],
    ));
    final share = ChatSessionShare.of(session, minInterval: Duration.zero);
    final owner = BroadcastViewLink('canoa.popout/test-a');
    share.serve(owner);
    final view = await ChatSessionView.attach(BroadcastViewLink('canoa.popout/test-a'), clientId: 'win', name: 'Window');

    expect(view.session.messages.single.text, 'Hello');
    expect(view.session.pendingPermissions.single.title, 'Edit a.txt');

    final question = view.session.pendingPermissions.single;
    view.choosePermission(question, question.options.last);
    await _later();
    expect(await answer, isA<acp.AcpSelect>().having((s) => s.optionId, 'option', 'no'));
    expect(view.session.pendingPermissions, isEmpty);
    expect(view.session.settledPermissions.single.outcome!.by, 'win');

    owner.send({'t': 'ended', 'reason': 'gone'});
    expect(await view.done, 'gone');
    await share.close();
  });

  test('a link on another name hears nothing of it', () async {
    final owner = BroadcastViewLink('canoa.popout/test-b');
    final stranger = BroadcastViewLink('canoa.popout/test-other');
    final heard = <Map<String, Object?>>[];
    stranger.messages.listen(heard.add);
    final listener = BroadcastViewLink('canoa.popout/test-b');
    final got = listener.messages.first;
    owner.send({'t': 'x'});
    expect((await got)['t'], 'x');
    await _later();
    expect(heard, isEmpty);
    await owner.close();
    await stranger.close();
    await listener.close();
  });

  test('a window with no name in its address is not a pop-out', () {
    expect(popoutLaunch(const []), isNull);
  });
}
