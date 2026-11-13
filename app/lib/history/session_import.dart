import 'package:coqui_acp/coqui_acp.dart' show AcpSessionInfo, AcpSessionPage;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show shortText;

import '../chat/agent_events.dart' show chatLimits;
import '../desktop/control_client.dart';

/// A source whose earlier sessions the host can bring in: the host's entry, the harness it runs, and how it is shown.
typedef ImportSource = ({String entry, String harness, String label});

/// One earlier session of a harness that the host found in the harness's own folder, for the person to choose.
final class ImportChoice {
  const ImportChoice({
    required this.source,
    required this.entry,
    required this.harness,
    required this.id,
    required this.title,
    this.cwd,
    this.imported = false,
  });

  /// The source as shown, and the host entry and harness it is.
  final String source, entry, harness;

  /// The harness's own id for the session, and the title the host took from the first thing said.
  final String id, title;
  final String? cwd;

  /// Whether the host already has it.
  final bool imported;
}

/// What an import brought, and what did not come or did not carry, as sentences.
final class ImportOutcome {
  const ImportOutcome({required this.imported, this.notes = const []});

  final int imported;
  final List<String> notes;
}

String? _text(Object? value) => shortText(value, chatLimits);

/// The sessions each of [sources] has to bring in, which the host reads its folders for when asked. Titles and folders only.
Future<List<ImportChoice>> listImportable(ControlClient control, List<ImportSource> sources) async {
  final out = <ImportChoice>[];
  for (final source in sources) {
    final answer = await control.call('history/importable', {'harness': source.harness, 'entry': source.entry});
    if (answer is! Map || answer['sessions'] is! List) throw StateError('the host sent no list of sessions');
    for (final each in (answer['sessions'] as List<Object?>).take(500)) {
      if (each is! Map || each['id'] is! String) continue;
      out.add(ImportChoice(
        source: source.label,
        entry: source.entry,
        harness: source.harness,
        id: each['id'] as String,
        title: _text(each['title']) ?? 'Untitled',
        cwd: _text(each['cwd']),
        imported: each['imported'] == true,
      ));
    }
  }
  return out;
}

/// Asks the host to bring in [chosen], one request per source, and says what came and what did not.
Future<ImportOutcome> importChosen(ControlClient control, List<ImportChoice> chosen) async {
  var imported = 0;
  final notes = <String>[];
  final groups = <(String, String), List<ImportChoice>>{};
  for (final each in chosen) {
    groups.putIfAbsent((each.harness, each.entry), () => []).add(each);
  }
  for (final MapEntry(key: (harness, entry), value: choices) in groups.entries) {
    final titles = {for (final each in choices) each.id: each.title};
    final answer = await control.call('history/import', {'harness': harness, 'entry': entry, 'ids': [for (final each in choices) each.id]});
    if (answer is! Map) throw StateError('the host answered an import with something that is no report');
    for (final done in (answer['imported'] as List<Object?>? ?? const []).take(500)) {
      if (done is! Map) continue;
      imported++;
      final skipped = done['notCarried'] is Map ? (done['notCarried'] as Map)['lines'] : null;
      if (skipped is int && skipped > 0) {
        final name = _text(done['title']) ?? titles[done['id']] ?? 'A session';
        notes.add('$name: $skipped line${skipped == 1 ? '' : 's'} of its file could not be read and did not carry.');
      }
    }
    for (final refused in (answer['refused'] as List<Object?>? ?? const []).take(500)) {
      if (refused is! Map) continue;
      final name = titles[refused['id']] ?? 'A session';
      notes.add('$name: not imported. ${_text(refused['why']) ?? 'The host gave no reason.'}');
    }
  }
  return ImportOutcome(imported: imported, notes: notes);
}

/// The sessions the host brought in under [entry], as Earlier conversations lists them. Their ids are the host's.
Future<List<AcpSessionInfo>> importedSessions(ControlClient control, String entry) async {
  final answer = await control.call('history/imported', {'entry': entry});
  if (answer is! Map || answer['sessions'] is! List) return const [];
  return [
    for (final each in (answer['sessions'] as List<Object?>).take(500))
      if (each is Map && each['session'] is String)
        AcpSessionInfo(
          id: each['session'] as String,
          cwd: _text(each['cwd']),
          title: _text(each['title']) ?? 'Imported session',
          updatedAt: _text(each['at']),
        ),
  ];
}

/// A page of Earlier conversations: what the host brought in comes first, on the first page, then what the agent itself lists.
/// [agent] is null for a connection that lists nothing, and a connection that fails to list still shows what was brought in.
Future<AcpSessionPage> earlierPage(
  String? cursor, {
  Future<List<AcpSessionInfo>> Function()? imported,
  Future<AcpSessionPage> Function(String? cursor)? agent,
  void Function(Iterable<String> ids)? onImported,
}) async {
  final brought = imported != null && cursor == null ? await imported() : const <AcpSessionInfo>[];
  onImported?.call(brought.map((each) => each.id));
  if (agent == null) return AcpSessionPage(brought, null);
  try {
    final page = await agent(cursor);
    return AcpSessionPage([...brought, ...page.sessions], page.nextCursor, incomplete: page.incomplete);
  } on Object {
    if (brought.isEmpty) rethrow;
    return AcpSessionPage(brought, null);
  }
}
