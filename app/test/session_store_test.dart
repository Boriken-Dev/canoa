import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('preview conversations remain independent', () {
    final store = SessionStore(includePreview: false);
    final first = store.createPreviewSession(title: 'First');
    store.sendPreviewText('Hello');
    final second = store.createPreviewSession(title: 'Second');
    store.sendPreviewText('World');

    expect(first.messages.first.text, 'Hello');
    expect(second.messages.first.text, 'World');
    store.select(first.id);
    expect(store.selected?.title, 'First');
    store.dispose();
  });

  test("the host's notices about an agent show when its session opens, bounded", () {
    final store = SessionStore(includePreview: false);
    final session = store.addProviderSession(
      title: 'pi', agent: 'pi', peerId: 'sha256:example', remoteSessionId: 's1',
    );
    store.applyOpened(session, AcpOpenedSession('s1', {
      'sessionId': 's1',
      '_meta': {
        SessionStore.hostNoticesKey: [
          'pi runs bash\u202e without asking',
          42,
          'Each message runs pi anew.',
          'x' * 2000,
          'four',
          'five',
        ],
      },
    }));
    final notices = session.messages.where((message) => message.role == MessageRole.notice).map((m) => m.text).toList();
    expect(notices, hasLength(3), reason: 'four taken, one of them not text');
    // Any agent can send these, so each says it is the agent's, never Canoa's own.
    expect(notices[0], 'The agent says: pi runs bash  without asking');
    expect(notices[1], 'The agent says: Each message runs pi anew.');
    expect(notices[2].length, lessThanOrEqualTo(513 + 'The agent says: '.length));

    final plain = store.addProviderSession(
      title: 'agent', agent: 'agent', peerId: 'sha256:example', remoteSessionId: 's2',
    );
    store.applyOpened(plain, AcpOpenedSession('s2', {'sessionId': 's2', '_meta': {SessionStore.hostNoticesKey: 'not a list'}}));
    expect(plain.messages, isEmpty);
    store.dispose();
  });

  test('remote ACP session identity is distinct from local view', () {
    final store = SessionStore(includePreview: false);
    final remote = store.addRemoteSession(
      title: 'Remote', agent: 'Example',
      peerId: 'sha256:example', remoteSessionId: 'agent-session-17',
      endpoint: Uri.parse('wss://example.org/acp'),
    );
    expect(remote.remoteSessionId, 'agent-session-17');
    expect(remote.id, isNot('agent-session-17'));
    expect(remote.status, SessionStatus.disconnected);
    expect(() => store.sendPreviewText('not connected'), throwsStateError);
    store.dispose();
  });

  test('selected page context is local, per-session and not sent', () {
    final store = SessionStore(includePreview: false);
    final first = store.createPreviewSession(title: 'First');
    final second = store.createPreviewSession(title: 'Second');
    store.addPageContext('{"url":"https://example.com/p",'
        '"title":"Page","selection":"Chosen"}');
    expect(second.messages.single.text, contains('Attached page context (not sent)'));
    expect(second.messages.single.text, contains('Chosen'));
    expect(first.messages, isEmpty);
    store.dispose();
  });

  test('malformed context is ignored', () {
    final store = SessionStore(includePreview: false);
    final session = store.createPreviewSession();
    store.addPageContext('not json');
    store.addPageContext('{"url":"javascript:alert(1)"}');
    expect(session.messages, isEmpty);
    store.dispose();
  });

  test('remote session stores WebTransport and same-host WSS fallback', () {
    final store = SessionStore(includePreview: false);
    final session = store.addRemoteSession(
      title: 'Remote', agent: 'Example',
      peerId: 'sha256:example', remoteSessionId: 'session-22',
      endpoint: Uri.parse('https://agent.example:4433/acp'),
      websocketFallback: Uri.parse('wss://agent.example:4434/acp'),
    );
    expect(session.endpoint!.scheme, 'https');
    expect(session.websocketFallback!.scheme, 'wss');
    expect(session.status, SessionStatus.disconnected);
    store.dispose();
  });

  test('session refuses fallback to a different host', () {
    final store = SessionStore(includePreview: false);
    expect(
      () => store.addRemoteSession(
        title: 'Remote', agent: 'Example',
        peerId: 'sha256:example', remoteSessionId: 'session-22',
        endpoint: Uri.parse('https://agent.example:4433/acp'),
        websocketFallback: Uri.parse('wss://other.example/acp'),
      ),
      throwsArgumentError,
    );
    store.dispose();
  });

  test('plain WS remote endpoints are rejected', () {
    final store = SessionStore(includePreview: false);
    expect(
      () => store.addRemoteSession(
        title: 'Insecure', agent: 'Example',
        peerId: 'sha256:example', remoteSessionId: 'session',
        endpoint: Uri.parse('ws://example.org/acp'),
      ),
      throwsArgumentError,
    );
    store.dispose();
  });
}
