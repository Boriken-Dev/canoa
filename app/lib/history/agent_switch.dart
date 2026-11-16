import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart';
import 'session_move.dart';

/// What the host measured of one source's conversations: whether its program can give one out in the standard history format and
/// take one in. A source that takes one in only as context ([replay]) is carried to as text in its first message.
final class Portability {
  const Portability({required this.export, required this.import, this.replay = false, this.why});

  final bool export, import, replay;

  /// Why the source cannot export or import, in the host's words; null when it can both.
  final String? why;

  /// A source whose host has nothing to say of it: nothing is assumed.
  static const unknown = Portability(export: false, import: false, why: 'the host did not say');

  static Portability? parse(Object? value) {
    if (value is! Map) return null;
    final why = value['why'];
    return Portability(
      export: value['export'] == true,
      import: value['import'] == true,
      replay: value['replay'] == true,
      why: why is String && why.isNotEmpty ? (why.length > 300 ? why.substring(0, 300) : why) : null,
    );
  }
}

/// What the host does to change a running conversation's agent: carry its history to another source, and say which sources can.
abstract interface class AgentMoves {
  /// Carries [session] of [entry] to the source [to]: its own export, imported there at a new session id.
  Future<MoveReport> move({required String entry, required String session, required String to, String? cwd});

  /// What each of the host's sources can export and import, by source name.
  Future<Map<String, Portability>> portability();
}

/// [AgentMoves] over the host's control channel.
final class ControlAgentMoves implements AgentMoves {
  ControlAgentMoves(this._control);

  final Future<ControlClient> Function() _control;

  @override
  Future<MoveReport> move({required String entry, required String session, required String to, String? cwd}) async =>
      requestMove(await _control(), entry: entry, session: session, to: to, cwd: cwd);

  @override
  Future<Map<String, Portability>> portability() async {
    final answer = await (await _control()).call('history/portable', const <String, Object?>{});
    final entries = answer is Map ? answer['entries'] : null;
    if (entries is! Map) return const {};
    final out = <String, Portability>{};
    for (final MapEntry(:key, :value) in entries.entries) {
      final parsed = Portability.parse(value);
      if (key is String && parsed != null) out[key] = parsed;
    }
    return out;
  }
}

/// What each source can do with another agent's conversation, asked of the host when the picker opens and kept.
class PortableAgents extends ChangeNotifier {
  PortableAgents(this._moves);

  final AgentMoves _moves;
  Map<String, Portability> _known = const {};
  bool _asked = false, _failed = false;

  /// What the host said of [entry]; null until it has said, or when it does not serve it as an agent of its own.
  Portability? of(String entry) => _known[entry];

  /// Whether the host has been asked yet.
  bool get asked => _asked;

  /// Whether the host could not answer, so nothing is known of any source.
  bool get failed => _failed;

  Future<void> refresh() async {
    try {
      _known = await _moves.portability();
      _failed = false;
    } on Object {
      // The host cannot say: nothing is offered that it did not promise.
      _known = const {};
      _failed = true;
    }
    _asked = true;
    notifyListeners();
  }
}

/// Why a source cannot take another agent's conversation, as the picker says it; null when it can.
String? cannotTake(Portability? portability, {required String name}) {
  if (portability == null) return '$name is not an agent this computer runs itself, so a conversation cannot be carried to it.';
  if (portability.import) return null;
  return '$name cannot take a conversation from another agent${portability.why == null ? '' : ': ${portability.why}'}';
}
