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
