import 'dart:async';
import 'dart:convert';

import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart';
import 'worktree.dart';

/// One request to a host on a connection of its own, its `result` or a [WorktreeFailure] with the host's words.
typedef HostWorktreeCall = Future<Map<String, Object?>> Function(String method, Map<String, Object?> params);

/// Worktrees `canoa-host` makes for the agents it serves, inside the working roots of their entry; the host decides where
/// and refuses anything outside them. Each request is a connection of its own, made before any agent starts on it, so a
/// worktree is removed with no agent working in it.
final class HostWorktrees implements Worktrees {
  const HostWorktrees(this.call);
  final HostWorktreeCall call;

  static String _text(Map<String, Object?> result, String key) => switch (result[key]) {
    final String text when text.isNotEmpty => text,
    _ => throw WorktreeFailure('The host answered no $key.'),
  };

  static int _count(Map<String, Object?> result, String key) => switch (result[key]) {
    final int count when count >= 0 => count,
    _ => throw WorktreeFailure('The host answered no $key.'),
  };

  @override
  Future<SessionWorktree> create(String cwd, String branch) async {
    final made = await call('_canoa/worktree/new', {'cwd': cwd, 'branch': branch});
    final worktree = SessionWorktree.fromJson(made);
    if (worktree == null || worktree.branch != branch) throw const WorktreeFailure('The host answered no worktree.');
    return worktree;
  }

  @override
  Future<WorktreeState> status(SessionWorktree worktree) async {
    final state = await call('_canoa/worktree/status', {'path': worktree.path});
    if (state['exists'] != true) return const WorktreeState.gone();
    final detached = state['detached'] == true;
    return WorktreeState(
      branch: detached ? null : _text(state, 'branch'),
      detached: detached,
      changed: _count(state, 'changed'),
      commits: _count(state, 'commits'),
      listing: _text(state, 'listing'),
    );
  }

  @override
  Future<WorktreeRemoval> remove(SessionWorktree worktree, {bool discard = false}) async {
    final removed = await call('_canoa/worktree/remove', {'path': worktree.path, 'discard': discard});
    return WorktreeRemoval(
      branch: _text(removed, 'branch'),
      branchKept: removed['branchKept'] == true,
      commits: _count(removed, 'commits'),
    );
  }
}

/// One worktree request on a connection of its own to [endpoints]: the peer is proven as [expectedPeerId] by [verifyPeer]
/// before anything is sent, and the connection closes with the answer. No `initialize` is sent, so no agent starts.
Future<Map<String, Object?>> hostWorktreeRequest(
  WebEndpoints endpoints, {
  required String expectedPeerId,
  required VerifyAcpPeer verifyPeer,
  required String method,
  required Map<String, Object?> params,
  Duration timeout = const Duration(seconds: 10),
  Duration answerTimeout = const Duration(minutes: 2),
}) async {
  final source = await connectWebChannel(endpoints, timeout: timeout);
  final staged = StagedChannel(source);
  WebChannel closing = staged;
  try {
    final proven = await verifyPeer(staged, expectedPeerId).timeout(timeout);
    closing = proven.channel;
    if (proven.peerId != expectedPeerId) throw const WorktreeFailure('Verified peer does not match expected identity');
    staged.allowApplicationMessages();
    final answer = proven.channel.messages
        .map((message) {
          try {
            return jsonDecode(message);
          } on FormatException {
            return null;
          }
        })
        .firstWhere((message) => message is Map && message['id'] == 1 && message['method'] == null)
        .timeout(answerTimeout);
    await proven.channel.send(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params}));
    final reply = await answer as Map;
    if (reply['error'] case final Map error) {
      throw WorktreeFailure('${error['message'] ?? 'The host refused it.'}');
    }
    final result = reply['result'];
    if (result is! Map) throw const WorktreeFailure('The host answered nothing.');
    return result.cast<String, Object?>();
  } on StateError {
    throw const WorktreeFailure('The host closed the connection without answering.');
  } on TimeoutException {
    throw const WorktreeFailure('The host did not answer in time.');
  } finally {
    await closing.close();
  }
}
