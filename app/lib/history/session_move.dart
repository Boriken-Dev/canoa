import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show shortText;

import '../acp/client.dart';
import '../chat/agent_events.dart' show chatLimits;
import '../desktop/control_client.dart';
import '../selectable.dart';
import '../session_store.dart';

/// One kind of loss the host reports for a move: what did not carry, as counts only.
final class MoveLoss {
  const MoveLoss(this.kind, this.count, this.detail);

  final String kind;
  final int count;
  final String? detail;
}

/// What the host did with a session it moved: the new session's id and what the target could not carry.
final class MoveReport {
  const MoveReport({
    required this.session,
    required this.entry,
    required this.cwd,
    required this.replayed,
    required this.compacted,
    required this.losses,
  });

  /// The new session, by the id the target's entry opens it with.
  final String session;
  final String entry;
  final String cwd;

  /// The target keeps no session of its own to seed: the history reaches its model in the first prompt.
  final bool replayed;
  final bool compacted;
  final List<MoveLoss> losses;

  /// A report the host sent, or null when it is not one: every field is checked, and a count is bounded.
  static MoveReport? parse(Object? result) {
    if (result is! Map) return null;
    final session = result['session'];
    final entry = result['entry'];
    final cwd = result['cwd'];
    if (session is! String || entry is! String || cwd is! String || session.isEmpty) return null;
    final losses = <MoveLoss>[];
    if (result['losses'] case final List<Object?> listed) {
      for (final each in listed.take(32)) {
        if (each is! Map || each['kind'] is! String || each['count'] is! int) continue;
        losses.add(MoveLoss(
          shortText(each['kind'], chatLimits) ?? 'unknown',
          (each['count'] as int).clamp(0, 1 << 40),
          shortText(each['detail'], chatLimits),
        ));
      }
    }
    return MoveReport(
      session: session,
      entry: entry,
      cwd: cwd,
      replayed: result['replayed'] == true,
      compacted: result['compacted'] == true,
      losses: losses,
    );
  }

  /// The loss of [kind], or null.
  MoveLoss? loss(String kind) => losses.where((each) => each.kind == kind).firstOrNull;
}

/// What a loss kind says in a sentence, for a person. An unknown kind is named as the host named it.
String lossText(MoveLoss loss) {
  final n = loss.count;
  final many = n == 1 ? '' : 's';
  final text = switch (loss.kind) {
    'thinking' => '$n thought$many left out: a model\'s reasoning is not carried between agents',
    'tool_as_note' => '$n tool call$many carried as a note: the other agent has no equivalent tool',
    'attachment_by_digest' => '$n attachment$many carried by its digest, not its content',
    'replayed_as_prompt' => 'carried as context in the first message, not as a conversation the agent keeps',
    'history_cut' => '$n earlier item$many cut to fit what the agent takes',
    'item_cut' => '$n long item$many shortened',
    'non_text_omitted' => '$n non-text item$many left out',
    'summary_missing' => 'a compaction had no summary, so the turns before it start from nothing',
    final other => '$other ($n)',
  };
  return loss.detail == null ? text : '$text. ${loss.detail}';
}

/// The note a moved session opens with: where it came from and what did not carry.
String moveSummary(MoveReport report, {required String from}) {
  final lines = <String>['Moved from $from to ${report.entry}. Nothing was run again: this is context, not commands.'];
  if (report.replayed) {
    lines.add('${report.entry} keeps no session to seed, so what was said reaches it in your first message here.');
  }
  if (report.compacted) lines.add('Earlier turns were left out, as they did not fit.');
  if (report.losses.isEmpty) {
    lines.add('Everything carried.');
  } else {
    lines.add('Did not carry:');
    for (final loss in report.losses) {
      lines.add('- ${lossText(loss)}');
    }
  }
  return lines.join('\n');
}

/// The conversation a history holds, as the person and the agent said it: prompts and the agent's messages. Tool calls,
/// thoughts and everything else are left to the target's own replay. Bounded: it reads what it is given and no more.
List<({MessageRole role, String text})> historyMessages(String history, {int limit = 2000}) {
  final out = <({MessageRole role, String text})>[];
  var agent = StringBuffer();
  void flush() {
    if (agent.isNotEmpty) out.add((role: MessageRole.agent, text: agent.toString()));
    agent = StringBuffer();
  }

  for (final line in const LineSplitter().convert(history).skip(1)) {
    if (out.length >= limit) break;
    final Object? entry;
    try {
      entry = jsonDecode(line);
    } on FormatException {
      continue;
    }
    if (entry is! Map) continue;
    final params = entry['params'];
    switch (entry['method']) {
      case 'session/prompt' when params is Map && params['prompt'] is List:
        flush();
        final text = [
          for (final block in params['prompt'] as List<Object?>)
            if (block is Map && block['type'] == 'text' && block['text'] is String) block['text'] as String,
        ].join('\n');
        if (text.isNotEmpty) out.add((role: MessageRole.user, text: text));
      case 'session/update' when params is Map && params['update'] is Map:
        final update = params['update'] as Map<Object?, Object?>;
        if (update['sessionUpdate'] == 'agent_message_chunk' &&
            update['content'] is Map &&
            (update['content'] as Map)['text'] is String) {
          agent.write((update['content'] as Map)['text'] as String);
        }
      case 'history/turn_end':
        flush();
    }
  }
  flush();
  return out;
}

/// What the host holds of a session, as one text: its pages joined, at most [pages] of them.
Future<String> sessionHistory(ControlClient control, {required String entry, required String session, int pages = 16}) async {
  final text = StringBuffer();
  String? after;
  for (var page = 0; page < pages; page++) {
    final answer = await control.call('history/export', {'entry': entry, 'session': session, 'after': ?after});
    if (answer is! Map || answer['text'] is! String) throw StateError('the host sent no history');
    text.write(answer['text']);
    after = answer['next'] as String?;
    if (after == null) break;
  }
  return text.toString();
}

/// Asks the host to move [session] of [entry] to the source [to].
Future<MoveReport> requestMove(
  ControlClient control, {
  required String entry,
  required String session,
  required String to,
  String? cwd,
}) async {
  final report = MoveReport.parse(await control.call('history/move', {
    'entry': entry,
    'session': session,
    'to': to,
    'cwd': ?cwd,
  }));
  if (report == null) throw StateError('the host answered a move with something that is no report');
  return report;
}

/// The source to move to, among [targets] (their names, and what each is shown as), or null.
Future<String?> pickTarget(BuildContext context, {required Map<String, String> targets}) => showSelectableDialog<String>(
  context: context,
  builder: (context) => SimpleDialog(
    key: const Key('move-target'),
    title: const Text('Move to…'),
    children: [
      for (final MapEntry(key: name, value: shown) in targets.entries)
        SimpleDialogOption(
          key: Key('move-to-$name'),
          onPressed: () => Navigator.pop(context, name),
          child: Text(shown),
        ),
    ],
  ),
);

/// Moves [session] to [to]: the host carries its history over, [connect] reaches the target's agent, and the new session opens
/// beside the old one with its history and what did not carry. The old session stays as it was.
Future<ChatSession> moveSession({
  required SessionStore store,
  required ControlClient control,
  required ChatSession session,
  required String to,
  required Future<VerifiedAgent> Function(String entry) connect,
  required void Function(VerifiedAgent client) keep,
}) async {
  final from = session.agent;
  final remote = session.remoteSessionId;
  if (remote == null || session.isDraft) throw StateError('That session has not started, so there is nothing to move');
  final report = await requestMove(control, entry: from, session: remote, to: to, cwd: session.cwd);
  final VerifiedAgent client = await connect(report.entry);
  try {
    // What the agent does not send back when the session opens is shown from the host's copy of the history.
    var carried = const <({MessageRole role, String text})>[];
    try {
      carried = historyMessages(await sessionHistory(control, entry: report.entry, session: report.session));
    } on Object {
      // The new session works without being shown what it carries.
    }
    final moved = await store.openMoved(
      session,
      client,
      sessionId: report.session,
      agent: report.entry,
      cwd: report.cwd,
      notes: [moveSummary(report, from: from)],
      carried: carried,
    );
    keep(client);
    return moved;
  } catch (_) {
    await client.close();
    rethrow;
  }
}
