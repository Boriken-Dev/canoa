/// What a session was given of its profile: the host prepares it, and its answers to `initialize` and to a session request
/// report each part under `_meta["dev.boriken.canoa/profile"]`. `docs/configuration.md`, "Applying a profile", has the table.
library;

import 'package:flutter/material.dart';

import 'profiles_panel.dart' show visible;

/// Where the host puts its report in an answer's `_meta`.
const hostProfileMeta = 'dev.boriken.canoa/profile';

/// Where `initialize` names the overlays a session starts with: a list of names, which only narrows what the host's profile lists.
const overlaysMeta = 'dev.boriken.canoa/overlays';

/// Each harness's name, by its AOS harness id.
const harnessLabels = {
  'claude-code': 'Claude Code',
  'codex': 'Codex',
  'gemini-cli': 'Gemini CLI',
  'pi': 'Pi',
  'opencode': 'OpenCode',
  'grok': 'Grok',
};

/// What became of one part of a profile.
enum ItemState {
  applied('applied'),
  unsupported('not supported'),
  off('off'),
  refused('refused');

  const ItemState(this.label);
  final String label;
}

/// One part of a profile ([key] such as `context`, `skills.review`, `mcp.github`, `env.TOKEN`, `model`), its [layer],
/// what became of it, and [how]: the channel it went through, or why it did not.
final class ProfileItem {
  const ProfileItem(this.key, this.layer, this.state, this.how);
  final String key, layer, how;
  final ItemState state;

  @override
  String toString() => '$key ($layer): ${state.label}, $how';
}

/// What a session was given of its profile, for the harness it went to.
final class ProfileReport {
  const ProfileReport(this.harness, this.items);

  /// The harness's name, or the agent entry's when Canoa does not know it.
  final String harness;
  final List<ProfileItem> items;

  Iterable<ProfileItem> get applied => items.where((item) => item.state == ItemState.applied);
  Iterable<ProfileItem> get notApplied => items.where((item) => item.state != ItemState.applied);

  /// One line for the conversation: how many applied, and which did not, by key.
  String get summary {
    final missed = notApplied.toList();
    return 'Profile for ${harnessLabels[harness] ?? harness}: ${applied.length} applied'
        '${missed.isEmpty ? '' : '; not applied: ${missed.map((item) => '${item.key} (${item.state.label})').join(', ')}'}';
  }
}

/// The host's report [meta] (an answer's `_meta[hostProfileMeta]`), or null when there is none or it is not one. [fallback] names
/// the harness when the host names none. The values of `model`, `effort` and `mode` are kept apart in [HostProfile.values].
HostProfile? hostProfileOf(Object? meta, {required String fallback}) {
  if (meta is! Map) return null;
  final items = meta['items'];
  if (items is! List) return null;
  final parsed = <ProfileItem>[];
  final values = <String, String>{};
  for (final each in items.take(512)) {
    if (each is! Map) continue;
    final key = each['key'], layer = each['layer'], state = each['state'], how = each['how'];
    if (key is! String || layer is! String || state is! String || how is! String) continue;
    final known = ItemState.values.where((candidate) => candidate.label == state).firstOrNull ?? ItemState.refused;
    parsed.add(ProfileItem(key, layer, known, how));
    if (each['value'] case final String value when const {'model', 'effort', 'mode'}.contains(key)) values[key] = value;
  }
  final harness = meta['harness'];
  return HostProfile(ProfileReport(harness is String && harness.isNotEmpty ? harness : fallback, parsed), values);
}

/// What the host reported of a profile: the report, and what it asks of the session's options.
final class HostProfile {
  const HostProfile(this.report, this.values);
  final ProfileReport report;

  /// The profile's `model`, `effort` and `mode`, by key: the host cannot set them, so the session's options do.
  final Map<String, String> values;
}

/// Profile items as rows: what, its layer, what became of it, and how or why.
class ProfileItemsTable extends StatelessWidget {
  const ProfileItemsTable({super.key, required this.items});
  final List<ProfileItem> items;

  @override
  Widget build(BuildContext context) {
    if (items.isEmpty) return const Text('Nothing else is set.');
    final scheme = Theme.of(context).colorScheme;
    return Table(
      columnWidths: const {
        0: IntrinsicColumnWidth(),
        1: IntrinsicColumnWidth(),
        2: IntrinsicColumnWidth(),
        3: FlexColumnWidth(),
      },
      children: [
        for (final item in items)
          TableRow(key: ValueKey('profile-item-${item.key}-${item.layer}'), children: [
            Padding(padding: const EdgeInsets.all(4), child: Text(visible(item.key))),
            Padding(padding: const EdgeInsets.all(4), child: Text(item.layer)),
            Padding(
              padding: const EdgeInsets.all(4),
              child: Text(item.state.label,
                  key: Key('profile-state-${item.key}'),
                  style: TextStyle(color: item.state == ItemState.applied ? null : scheme.error)),
            ),
            Padding(padding: const EdgeInsets.all(4), child: Text(visible(item.how))),
          ]),
      ],
    );
  }
}

/// What a session was given of its profile, from the conversation's header.
class ProfileReportDialog extends StatelessWidget {
  const ProfileReportDialog({super.key, required this.report});
  final ProfileReport report;

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text('Profile for ${harnessLabels[report.harness] ?? report.harness}'),
    content: SizedBox(width: 640, child: SingleChildScrollView(child: ProfileItemsTable(items: report.items))),
    actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
  );
}
