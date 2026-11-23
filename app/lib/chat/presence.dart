import 'package:material_ui/material_ui.dart';

import '../acp/hub_wire.dart' show HubClient;

/// Says in a conversation's header who else is attached to the session, which the host keeps running for several clients: each
/// is a device or window that sees what is said and may answer for the agent.
class PresenceMark extends StatelessWidget {
  const PresenceMark({super.key, required this.others, required this.narrow});

  /// The clients attached besides this one.
  final List<HubClient> others;
  final bool narrow;

  /// What the tooltip says, naming each client.
  static String message(List<HubClient> others) => others.length == 1
      ? 'Also attached: ${others.first.named}'
      : 'Also attached: ${others.map((each) => each.named).join(', ')}';

  /// The words on the chip.
  static String label(List<HubClient> others) =>
      others.length == 1 ? 'With ${others.first.named}' : 'With ${others.length} others';

  @override
  Widget build(BuildContext context) {
    if (others.isEmpty) return const SizedBox.shrink();
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: message(others),
      child: Padding(
        key: const Key('presence-mark'),
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: narrow
            ? Icon(Icons.group_outlined, size: 18, color: scheme.primary)
            : Chip(
                avatar: Icon(Icons.group_outlined, size: 16, color: scheme.onSecondaryContainer),
                label: Text(label(others), style: TextStyle(color: scheme.onSecondaryContainer), overflow: TextOverflow.ellipsis),
                backgroundColor: scheme.secondaryContainer,
                side: BorderSide.none,
                visualDensity: VisualDensity.compact,
              ),
      ),
    );
  }
}
