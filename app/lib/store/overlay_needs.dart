/// What an installed overlay needs of this computer, as the host checks it: the programs and versions its `runtime` names, and
/// the interpreters its scripts run under. The host looks on its own `PATH` and in the Store's runtimes, so the answer is the one a
/// session would get; a program that is missing carries the Store runtime that installs it.
library;

import '../desktop/host_control.dart';

/// One thing an overlay needs and whether this computer has it.
final class NeedLine {
  const NeedLine({required this.key, required this.met, required this.how, this.offer});

  /// `runtime.python`, `runtime.commands: git`, `dependencies: lib/python/requirements.txt`, ...
  final String key;

  /// Whether this computer has it, or it is prepared.
  final bool met;

  /// Where it was found, or what is missing.
  final String how;

  /// The Store runtime (a provider id such as `python_store`) that installs it, when it is missing and one does.
  final String? offer;

  @override
  String toString() => '$key: $how';
}

/// The needs of the installed overlay [overlay], in the order the host lists them; none when it cannot say.
typedef OverlayNeeds = Future<List<NeedLine>> Function(String overlay);

/// Asks the host's `overlay/needs`. A host that is down, refuses or does not have the method says nothing, which is no need.
OverlayNeeds controlOverlayNeeds(HostControl control) => (overlay) async {
  try {
    final client = await control.ready();
    final answer = await client.call('overlay/needs', {'name': overlay}, null, const Duration(seconds: 40));
    final items = answer is Map ? answer['items'] : null;
    if (items is! List) return const [];
    return [
      for (final item in items)
        if (item is Map && item['key'] is String && item['how'] is String)
          NeedLine(
            key: item['key']! as String,
            met: item['state'] == 'applied',
            how: item['how']! as String,
            offer: item['offer'] is String ? item['offer']! as String : null,
          ),
    ];
  } on Object {
    return const [];
  }
};
