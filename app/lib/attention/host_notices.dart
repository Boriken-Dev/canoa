import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show shortLine;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import 'notifier.dart';

/// The conversation a notice is about, as this app knows it.
typedef LocalSession = ({String id, String title});

/// Takes the notices the host sends over its control link and shows them through the [Notifier], and tells the host which
/// conversation the person is looking at so that it tells them nothing of that one, through any notifier.
///
/// The host decides which events happen, who is told and what the words are; nothing here chooses. A host that does not let this
/// link have notices (a key without the grant) is left alone.
final class HostNotices {
  HostNotices({required this.control, required this.notifier, required this.language, required this.localSession});

  final HostControl control;
  final Notifier notifier;

  /// The language the host words a notice in (`en`, `es`).
  final String Function() language;

  /// The conversation of this app that the agent's session [remote] is, when there is one.
  final LocalSession? Function(String remote) localSession;

  StreamSubscription<ControlPush>? _pushes;
  StreamSubscription<void>? _again;
  bool _started = false, _stopped = false;
  bool _focused = false;
  String? _selected;
  String? _told;

  /// Starts listening, and again each time the link to the host is made anew.
  void start() {
    if (_started) return;
    _started = true;
    _again = control.reconnected.listen((_) => unawaited(_attach()));
    unawaited(_attach());
  }

  /// Whether the person is looking at the app's window, from the platform.
  set focused(bool value) {
    _focused = value;
    unawaited(_report());
  }

  /// The agent's id of the session the app shows, or null for none.
  set selected(String? remote) {
    _selected = remote;
    unawaited(_report());
  }

  Future<void> _attach() async {
    if (_stopped) return;
    try {
      final client = await control.ready(wait: false);
      await _pushes?.cancel();
      _pushes = client.pushes.listen(_heard);
      // A new link has told the host nothing yet.
      _told = null;
      await client.call('notify/subscribe', {'language': language()});
      await _report();
    } on Object {
      // A host that gives this link no notices, or none yet: the person is not told through this window.
    }
  }

  Future<void> _report() async {
    final session = _focused ? _selected : null;
    if (session == _told) return;
    try {
      final client = await control.ready(wait: false);
      await client.call('notify/attended', {'session': session});
      _told = session;
    } on Object {
      // Not subscribed, or no link: the next report tries again.
    }
  }

  void _heard(ControlPush push) {
    if (push.method != 'notify/notice') return;
    final params = push.params;
    final remote = params['session'] is Map ? (params['session']! as Map)['id'] : null;
    final held = remote is String ? localSession(remote) : null;
    final said = params['session'] is Map ? (params['session']! as Map)['title'] : null;
    // A conversation the host knew by no title is called what this app calls it.
    final title = held != null && said is! String ? (shortLine(held.title) ?? '${params['title']}') : '${params['title']}';
    unawaited(notifier.show(title: title, body: '${params['body']}', sessionId: held?.id ?? (remote is String ? remote : '')));
  }

  Future<void> stop() async {
    _stopped = true;
    await _pushes?.cancel();
    await _again?.cancel();
  }
}
