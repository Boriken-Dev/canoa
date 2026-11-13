import 'dart:async';

import 'package:flutter/material.dart';

import '../profile/profile.dart';
import '../profile/profiles_dialog.dart' show visible;
import '../selectable.dart';
import '../store/overlay_page.dart' show oneLine;
import 'loadout_controller.dart';
import 'loadout_page.dart';
import 'mcp_registry.dart';

/// The MCPs section: the MCP servers the profile defines, and the servers a registry lists.
///
/// A registry is the MCP Registry's API (modelcontextprotocol/registry): each server is a `server.json`. Installing one writes a
/// server into the person's own profile and turns it off for every agent, so nothing starts until it is allowed here; what it
/// would write, and the digest of the `server.json` that was shown, are in front of the person before they say yes.
class McpsSection extends StatefulWidget {
  const McpsSection({super.key, required this.controller});
  final LoadoutController controller;

  @override
  State<McpsSection> createState() => _McpsSectionState();
}

class _McpsSectionState extends State<McpsSection> with AutomaticKeepAliveClientMixin {
  bool discover = false;
  final query = TextEditingController();

  LoadoutController get c => widget.controller;

  @override
  bool get wantKeepAlive => true;

  @override
  void dispose() {
    query.dispose();
    super.dispose();
  }

  void _toDiscover(bool value) {
    setState(() => discover = value);
    if (value && !c.registryRead && !c.loadingRegistry) unawaited(c.loadRegistry());
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) => ListView(padding: const EdgeInsets.all(16), children: [
        Align(
          alignment: Alignment.centerLeft,
          child: InstalledOrDiscover(
            section: 'mcps',
            discover: discover,
            installedCount: c.installedMcp().length,
            discoverCount: c.registryRead ? c.registryEntries.length : null,
            onChanged: _toDiscover,
          ),
        ),
        const SizedBox(height: 12),
        if (c.problem != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(visible(c.problem!), key: const Key('mcps-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        if (discover) ..._discover(context) else ..._installed(context),
      ]),
    );
  }

  List<Widget> _installed(BuildContext context) {
    final servers = c.installedMcp();
    if (servers.isEmpty) {
      return [
        const Text('No MCP servers yet. An MCP server gives an agent tools; none starts until you allow it.', key: Key('mcps-none')),
        const SizedBox(height: 8),
        FilledButton.tonal(key: const Key('mcps-to-discover'), onPressed: () => _toDiscover(true), child: const Text('Discover MCP servers')),
      ];
    }
    return [
      const Text('The servers your profile and your overlays define. One of yours starts only for an agent it is allowed for; an '
          'overlay\'s asks you when a session starts.'),
      const SizedBox(height: 8),
      for (final server in servers) _installedCard(context, server),
    ];
  }

  static String _describe(McpServer server) => switch (server) {
    McpStdio(:final command, :final args) => 'Starts $command${args.isEmpty ? '' : ' ${args.join(' ')}'}',
    McpHttp(:final url, :final sse) => 'Connects to $url${sse ? ' (SSE)' : ''}',
  };

  Widget _installedCard(BuildContext context, InstalledMcp server) {
    final origin = server.origin;
    final allowed = server.fromOverlay ? 0 : c.allowedFor(server.name);
    final total = c.agents.length;
    final text = Theme.of(context).textTheme;
    return LoadoutCard(
      key: Key('mcp-installed-${server.name}'),
      leading: const LoadoutMark(name: 'mcp', icon: Icons.hub_outlined),
      title: server.name,
      lines: [
        _describe(server.server),
        if (server.fromOverlay) 'Off until you allow it, when a session asks' else if (total == 0) 'No agent to allow it for' else
          allowed == 0 ? 'Off for every agent' : 'Allowed for $allowed of $total agents',
      ],
      provenance: origin != null
          ? 'from the registry ${Uri.tryParse(origin.registry)?.host ?? origin.registry} · ${origin.server} ${origin.version} · ${_short(origin.digest)}'
          : server.fromOverlay ? 'comes with the overlay ${server.layer.substring(8)}' : 'written in canoa.cfg by hand',
      trailing: server.fromOverlay || total == 0
          ? null
          : Column(crossAxisAlignment: CrossAxisAlignment.end, children: [
              Switch(
                key: Key('mcp-allowed-${server.name}'),
                value: allowed == total,
                onChanged: (value) => unawaited(c.setMcpAllowed(server.name, value)),
              ),
              Text(allowed == total ? 'Allowed' : 'Off', style: text.labelSmall),
            ]),
      footer: server.fromOverlay
          ? null
          : Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                key: Key('mcp-remove-${server.name}'),
                onPressed: c.busy != null ? null : () => unawaited(c.removeMcp(server.name)),
                child: const Text('Remove'),
              ),
            ),
    );
  }

  static String _short(String digest) => digest.length > 19 ? '${digest.substring(0, 19)}…' : digest;

  List<Widget> _discover(BuildContext context) {
    final host = Uri.tryParse(c.registry)?.host ?? c.registry;
    return [
      Row(children: [
        Expanded(
          child: TextField(
            key: const Key('mcps-search'),
            controller: query,
            decoration: const InputDecoration(prefixIcon: Icon(Icons.search), labelText: 'Search the registry'),
            onSubmitted: (_) => unawaited(c.loadRegistry(search: query.text)),
          ),
        ),
        const SizedBox(width: 8),
        FilledButton.tonal(key: const Key('mcps-search-go'), onPressed: c.loadingRegistry ? null : () => unawaited(c.loadRegistry(search: query.text)), child: const Text('Search')),
      ]),
      const SizedBox(height: 4),
      Text('Listed by $host. A registry\'s listing is its publishers\' claim: what installing would write is shown from the server\'s '
          'own description, and nothing runs until you allow it.', style: Theme.of(context).textTheme.bodySmall),
      const SizedBox(height: 8),
      if (c.loadingRegistry) const LinearProgressIndicator(),
      if (c.registryProblem case final problem?)
        Text(visible(problem), key: const Key('mcps-registry-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
      if (c.registryRead && !c.loadingRegistry && c.registryProblem == null && c.registryEntries.isEmpty)
        const Text('The registry lists nothing for that.', key: Key('mcps-nothing')),
      for (final entry in c.registryEntries) _entryCard(context, entry, host),
      if (c.registryNext != null && !c.loadingRegistry)
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(key: const Key('mcps-more'), onPressed: () => unawaited(c.loadRegistry(more: true)), child: const Text('Show more')),
        ),
    ];
  }

  Widget _entryCard(BuildContext context, McpRegistryEntry entry, String host) {
    final installed = c.installedMcp().any((server) => server.origin?.server == entry.name);
    return LoadoutCard(
      key: Key('mcp-entry-${entry.name}'),
      leading: const LoadoutMark(name: 'mcp', icon: Icons.hub_outlined),
      title: entry.title,
      lines: [if (entry.description.isNotEmpty) entry.description, if (installed) 'Installed'],
      provenance: 'from the registry $host · ${entry.name} ${entry.version}${entry.repositoryUrl == null ? '' : ' · ${entry.repositoryUrl}'}',
      onTap: () => unawaited(_review(entry)),
    );
  }

  /// Shows what installing would write, from the `server.json` fetched, and writes it only on a yes.
  Future<void> _review(McpRegistryEntry entry) async {
    final options = optionsOf(entry.server);
    final found = <String, bool>{};
    for (final option in options) {
      final runtime = option.runtime;
      if (runtime != null && !found.containsKey(runtime)) found[runtime] = await c.findProgram?.call(runtime) != null;
    }
    if (!mounted) return;
    final picked = await showSelectableDialog<(McpOption, String)>(
      context: context,
      builder: (_) => McpInstallDialog(
        entry: entry,
        options: options,
        runtimeFound: found,
        registry: Uri.tryParse(c.registry)?.host ?? c.registry,
        taken: {...c.config.profile.mcp.keys},
        agents: c.agents,
      ),
    );
    if (picked == null) return;
    await c.run('Installing ${entry.title}', () async {
      await c.installMcp(entry, picked.$1, picked.$2);
    });
    if (mounted) setState(() => discover = false);
  }
}

/// What installing one registry server would write, with each way to run it, what each needs of this computer, and why Canoa
/// refuses the ones it does.
class McpInstallDialog extends StatefulWidget {
  const McpInstallDialog({super.key, required this.entry, required this.options, required this.runtimeFound,
      required this.registry, required this.taken, required this.agents});

  final McpRegistryEntry entry;
  final List<McpOption> options;

  /// Whether each runtime a package needs is on this computer.
  final Map<String, bool> runtimeFound;
  final String registry;

  /// The names `[profile.mcp]` already holds.
  final Set<String> taken;
  final List<String> agents;

  @override
  State<McpInstallDialog> createState() => _McpInstallDialogState();
}

class _McpInstallDialogState extends State<McpInstallDialog> {
  late int chosen = widget.options.indexWhere(_installable).clamp(0, widget.options.isEmpty ? 0 : widget.options.length - 1);
  late final TextEditingController name = TextEditingController(text: widget.entry.installName);

  bool _installable(McpOption option) => option.refusal == null && (option.runtime == null || widget.runtimeFound[option.runtime] == true);

  String? _whyNot(McpOption option) {
    if (option.refusal case final why?) return 'Not installed by Canoa: $why';
    if (option.runtime case final runtime? when widget.runtimeFound[runtime] != true) {
      return 'Needs $runtime, which is not on this computer. Canoa installs no runtime.';
    }
    return null;
  }

  @override
  void dispose() {
    name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final entry = widget.entry;
    final nameProblem = !isEntryName(name.text)
        ? 'A name has 1 to 64 letters, digits, - or _'
        : widget.taken.contains(name.text) ? 'canoa.cfg already has a server named ${name.text}' : null;
    final option = widget.options.isEmpty ? null : widget.options[chosen];
    return AlertDialog(
      key: const Key('mcp-install-dialog'),
      title: Text('Install ${oneLine(entry.title)}?'),
      content: SizedBox(
        width: 560,
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            Text('What the registry ${widget.registry} says', style: text.titleSmall),
            Text(oneLine('${entry.name} ${entry.version}'), key: const Key('mcp-claims')),
            if (entry.description.isNotEmpty) Text(oneLine(entry.description)),
            if (entry.repositoryUrl != null) Text(oneLine('Repository ${entry.repositoryUrl}')),
            if (entry.websiteUrl != null) Text(oneLine('Website ${entry.websiteUrl}')),
            Text('server.json digest ${entry.digest}', key: const Key('mcp-digest'), style: text.bodySmall),
            const Divider(height: 24),
            if (widget.options.isEmpty)
              const Text('It lists no package and no remote address, so there is nothing to install.', key: Key('mcp-no-options')),
            RadioGroup<int>(
              groupValue: chosen,
              onChanged: (value) => setState(() => chosen = value ?? chosen),
              child: Column(children: [
                for (final (index, each) in widget.options.indexed)
                  RadioListTile<int>(
                    key: Key('mcp-option-$index'),
                    dense: true,
                    enabled: _installable(each),
                    contentPadding: EdgeInsets.zero,
                    value: index,
                    title: Text(oneLine(each.label)),
                    subtitle: Text(oneLine([
                      if (each.commandLine != null) (each.kind == 'remote' ? 'Connects to ${each.commandLine}' : 'Starts ${each.commandLine}'),
                      ?_whyNot(each),
                    ].join('\n')), key: Key('mcp-option-text-$index')),
                  ),
              ]),
            ),
            if (option != null && option.refusal == null) ...[
              for (final need in option.needs) Text(oneLine(need.describe()), key: Key('mcp-need-${need.name}')),
              const SizedBox(height: 8),
              TextField(
                key: const Key('mcp-name'),
                controller: name,
                onChanged: (_) => setState(() {}),
                decoration: InputDecoration(labelText: 'Name in canoa.cfg', errorText: nameProblem),
              ),
            ],
            const SizedBox(height: 8),
            Text(widget.agents.isEmpty
                ? 'It is written to your profile, and off until you allow it in Loadout.'
                : 'It is written to your profile and turned off for ${widget.agents.join(', ')}: nothing starts until you allow it in Loadout.',
                key: const Key('mcp-off-note')),
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(
          key: const Key('mcp-install-confirm'),
          onPressed: option == null || !_installable(option) || nameProblem != null ? null : () => Navigator.pop(context, (option, name.text)),
          child: const Text('Install, off until allowed'),
        ),
      ],
    );
  }
}
