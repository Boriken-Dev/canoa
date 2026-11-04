import 'package:coqui_acp/coqui_acp.dart' show AcpMcpTransport;
import 'package:flutter/material.dart';

import 'apply.dart';
import 'profile_layers.dart';
import 'profiles_dialog.dart' show visible;

/// What the person chose before a session starts: the MCP servers to start, and whether to keep that for the agent.
final class ProfileStartChoice {
  const ProfileStartChoice(this.on, {this.remember = false});
  final Set<String> on;
  final bool remember;
}

/// What Canoa knows each harness takes with a session (`docs/configuration.md`, "Applying a profile"), for saying
/// before it starts what will apply.
AgentTakes expectedTakes(String? harness) => switch (harness) {
  'claude-code' => const AgentTakes(mcp: _anyMcp, systemPrompt: true, additionalDirectories: true),
  'codex' => const AgentTakes(mcp: _noSse, systemPrompt: true, additionalDirectories: true),
  'pi' => const AgentTakes(mcp: _stdioOnly),
  _ => const AgentTakes(mcp: _anyMcp),
};

bool _anyMcp(AcpMcpTransport transport) => true;
bool _noSse(AcpMcpTransport transport) => transport != AcpMcpTransport.sse;
bool _stdioOnly(AcpMcpTransport transport) => transport == AcpMcpTransport.stdio;

final class _NothingWritten implements SessionFiles {
  const _NothingWritten();
  @override
  Future<String> write(Map<String, List<int>> files) async => '(a new temporary directory)';
  @override
  Future<void> remove(String directory) async {}
}

/// What [prepared] would give [harness], worked out before anything starts and with nothing written.
Future<List<ProfileItem>> expectedItems(PreparedProfile prepared, String? harness, Set<String> on) async =>
    (await launchFor(prepared, harness: harness, takes: expectedTakes(harness), mcpOn: on, files: const _NothingWritten()))
        .items;

/// Shown before a session starts with a profile: every part of it, what [harness] is expected to take of it and what
/// it is not, the layers left out, and each MCP server (what it runs or reaches and the names it receives, never their
/// values) with a switch. A server of an overlay or a project is off until the person allows it.
class ProfileStartDialog extends StatefulWidget {
  const ProfileStartDialog({
    super.key,
    required this.agent,
    required this.prepared,
    this.harness,
    this.remembered = const {},
    this.notes = const [],
  });

  final String agent;
  final PreparedProfile prepared;
  final String? harness;

  /// `[agents.NAME.mcp]` of this agent.
  final Map<String, Object> remembered;
  final List<LayerNote> notes;

  @override
  State<ProfileStartDialog> createState() => _ProfileStartDialogState();
}

class _ProfileStartDialogState extends State<ProfileStartDialog> {
  late final Set<String> on = {
    for (final server in widget.prepared.mcp)
      if (mcpOnByDefault(server, widget.remembered)) server.name,
  };
  bool remember = false;
  late Future<List<ProfileItem>> items = expectedItems(widget.prepared, widget.harness, on);

  void _toggle(String name, bool value) => setState(() {
    value ? on.add(name) : on.remove(name);
    items = expectedItems(widget.prepared, widget.harness, on);
  });

  @override
  Widget build(BuildContext context) {
    final small = Theme.of(context).textTheme.bodySmall;
    final harness = harnessLabels[widget.harness];
    return AlertDialog(
      title: Text('Start ${widget.agent} with its profile?'),
      content: SizedBox(width: 640, child: SingleChildScrollView(child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(harness == null
              ? 'Canoa does not know this harness: what it takes is decided by what it says when it starts, and the '
                  'conversation then says what applied.'
              : 'What $harness takes of it, as Canoa knows it, and what it does not. The conversation says what applied '
                  'once it starts.'),
          const SizedBox(height: 8),
          FutureBuilder<List<ProfileItem>>(
            future: items,
            builder: (context, snapshot) => snapshot.data == null
                ? const LinearProgressIndicator()
                : ProfileItemsTable(items: [
                    for (final item in snapshot.data!)
                      if (!item.key.startsWith('mcp.')) item,
                  ]),
          ),
          if (widget.prepared.mcp.isNotEmpty) ...[
            const SizedBox(height: 12),
            Text('MCP servers', style: Theme.of(context).textTheme.titleMedium),
            for (final server in widget.prepared.mcp)
              CheckboxListTile(
                key: Key('profile-start-mcp-${server.name}'),
                dense: true,
                value: on.contains(server.name),
                onChanged: server.refused != null ? null : (value) => _toggle(server.name, value ?? false),
                title: Text(visible('${server.name}  (${server.layer})')),
                subtitle: Text(visible([
                  server.shown,
                  server.receives.isEmpty
                      ? 'Receives nothing from the profile'
                      : 'Receives ${server.isHttp ? 'headers' : 'variables'}: '
                          '${server.receives.join(', ')}',
                  for (final withheld in server.withheld) 'Left out: $withheld',
                  ...server.notes,
                  if (server.refused case final why?) 'Cannot start: $why',
                  if (server.confined && server.refused == null)
                    mcpOnByDefault(server, widget.remembered)
                        ? 'From ${server.layer}: allowed for ${widget.agent} at this definition'
                        : 'From ${server.layer}: off until you allow it',
                ].join('\n')), style: small),
              ),
            CheckboxListTile(
              key: const Key('profile-start-remember'),
              dense: true,
              value: remember,
              onChanged: (value) => setState(() => remember = value ?? false),
              title: Text('Keep these choices for ${widget.agent}'),
            ),
          ],
          for (final note in widget.notes) Text(visible('${note.layer}: ${note.text}'), style: small),
        ],
      ))),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(
          key: const Key('profile-start'),
          onPressed: () => Navigator.pop(context, ProfileStartChoice({...on}, remember: remember)),
          child: const Text('Start'),
        ),
      ],
    );
  }
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
