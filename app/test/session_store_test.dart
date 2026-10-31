import 'package:canoa/session_store.dart';
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
