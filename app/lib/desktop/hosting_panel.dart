import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';

import '../acp/agent_commands.dart';
import '../acp/share_path.dart';
import '../config/canoa_config.dart';
import '../forms/schema_fields.dart';
import '../providers/provider_catalog.dart' show EntryForm, providerCatalog;
import '../settings/setting_layers.dart' show SettingNote;
import '../theme/look.dart';
import 'host_supervisor.dart';
import 'host_tool.dart';
import '../selectable.dart';
import '../sources/source_icons.dart';

/// Checks and saves a changed canoa.cfg, then serves it: what is wrong with it, or null once it is saved.
typedef SaveHosting = Future<String?> Function(CanoaConfig changed);

/// What this computer hosts for browsers: the host's state, its agents, adding one, and the keys that may connect, each
/// paired or removed only by the person's own click on a dialog that shows the whole id.
class HostingPanel extends StatefulWidget {
  const HostingPanel({
    super.key,
    required this.supervisor,
    required this.config,
    required this.onSave,
    this.freePort = freeLoopbackPort,
  });

  final HostSupervisor supervisor;
  final CanoaConfig config;
  final SaveHosting onSave;
  final Future<int> Function() freePort;

  @override
  State<HostingPanel> createState() => _HostingPanelState();
}

class _HostingPanelState extends State<HostingPanel> {
  late CanoaConfig config = widget.config;
  late Future<List<HostKey>> keys = _keys();
  HostPairing? pairing;
  String? paired;
  String? message;
  bool busy = false;

  Future<List<HostKey>> _keys() => !widget.supervisor.managesKeys ? Future.value(const []) : widget.supervisor.keys();

  void _refresh() => setState(() {
    keys = _keys();
  });

  @override
  void dispose() {
    pairing?.cancel();
    super.dispose();
  }

  Future<void> _run(Future<void> Function() step) async {
    if (busy) return;
    setState(() {
      busy = true;
      message = null;
    });
    try {
      await step();
    } on Object catch (error) {
      if (mounted) setState(() => message = '$error');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> _save(CanoaConfig changed) async {
    final problem = await widget.onSave(changed);
    if (problem != null) throw HostCommandFailed(problem);
    if (mounted) setState(() => config = changed);
  }

  Future<void> _add() async {
    final added = await showSelectableDialog<Map<String, Object?>>(context: context, builder: (_) => const _AddAgentDialog());
    if (added == null) return;
    await _run(() async {
      final port = await widget.freePort();
      final program = added['program'] as String?;
      final args = [if (added['args'] is List) for (final each in added['args'] as List) '$each'];
      final roots = [if (added['working_roots'] is List) for (final each in added['working_roots'] as List) '$each'];
      // The keys the form has no special place for are the entry's own, as the host described them.
      const placed = {'name', 'program', 'args', 'working_roots', 'usage'};
      await _save(
        config.withHostAgent(
          name: '${added['name']}',
          program: program,
          args: args,
          listen: '127.0.0.1:$port',
          workingRoots: roots,
          usage: added['usage'] as String? ?? (program == null ? null : usageFor(AgentCommand(label: program, program: program, args: args))),
          more: {for (final MapEntry(:key, :value) in added.entries) if (!placed.contains(key)) key: value},
        ),
      );
    });
  }

  Future<void> _remove(HostEntry entry) async {
    final sure = await _confirm(
      title: 'Stop hosting ${entry.name}?',
      body: [Text('Browsers can no longer reach ${entry.name} through this computer, and a link to it now ends.')],
      action: 'Stop hosting',
    );
    if (sure) await _run(() => _save(config.withoutHostEntry(entry.name)));
  }

  Future<void> _accept(HostKey key) async {
    final sure = await _confirm(
      title: 'Pair this key?',
      body: [
        const Text(
          'Pair it only if it is exactly the id Canoa shows where you connected from, character for character. '
          'Anyone on this computer can make a key wait here.',
        ),
        const SizedBox(height: 12),
        _Id(key.id),
        if (key.from != null)
          Padding(padding: const EdgeInsets.only(top: 8), child: Text('It connected from ${key.from}.')),
      ],
      action: 'Pair',
    );
    if (sure) {
      await _run(() => widget.supervisor.accept(key.id));
      _refresh();
    }
  }

  Future<void> _revoke(HostKey key) async {
    final sure = await _confirm(
      title: key.accepted ? 'Remove this key?' : 'Turn this key away?',
      body: [
        _Id(key.id),
        const SizedBox(height: 12),
        Text(key.accepted ? 'Its links end within a second, and it has to be paired again.' : 'It stops waiting here.'),
      ],
      action: 'Remove',
    );
    if (sure) {
      await _run(() => widget.supervisor.revoke(key.id));
      _refresh();
    }
  }

  Future<void> _showPin({bool browser = false}) async {
    HostPairing? started;
    await _run(() async {
      pairing?.cancel();
      started = browser ? await widget.supervisor.pairBrowser() : await widget.supervisor.pair();
    });
    final running = started;
    if (running == null) return;
    if (!mounted) return running.cancel();
    setState(() {
      pairing = running;
      paired = null;
    });
    try {
      final who = await running.paired;
      if (!mounted) return;
      setState(() {
        paired = who;
        pairing = null;
      });
      _refresh();
    } on HostCommandFailed catch (error) {
      if (mounted && pairing == running) {
        setState(() {
          pairing = null;
          message = 'Pairing ended: $error';
        });
      }
    }
  }

  void _cancelPin() {
    final running = pairing;
    setState(() => pairing = null);
    running?.cancel();
  }

  Future<bool> _confirm({required String title, required List<Widget> body, required String action}) async =>
      await showSelectableDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(title),
          content: SizedBox(
            width: 460,
            child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: body),
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: Text(action)),
          ],
        ),
      ) ??
      false;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return ListenableBuilder(
      listenable: widget.supervisor,
      builder: (context, _) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _Status(supervisor: widget.supervisor),
          const SizedBox(height: 16),
          Row(
            children: [
              Expanded(child: Text('Agents this computer hosts', style: text.titleSmall)),
              TextButton.icon(
                onPressed: busy || !widget.supervisor.managesKeys ? null : _add,
                icon: const Icon(Icons.add),
                label: const Text('Add an agent'),
              ),
            ],
          ),
          if (config.hostEntries.isEmpty)
            const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('None yet.')),
          for (final entry in config.hostEntries) _entryTile(entry),
          const Divider(height: 32),
          Row(
            children: [
              Expanded(child: Text('Keys that may connect', style: text.titleSmall)),
              IconButton(
                tooltip: 'Look again',
                onPressed: busy ? null : _refresh,
                icon: const Icon(Icons.refresh),
              ),
            ],
          ),
          const Text('A browser page pairs by connecting once: its key then waits here until you pair it.'),
          FutureBuilder<List<HostKey>>(
            future: keys,
            builder: (context, snapshot) {
              if (snapshot.hasError) return Text('The keys could not be read: ${snapshot.error}');
              final found = snapshot.data;
              if (found == null) return const LinearProgressIndicator();
              if (found.isEmpty) {
                return const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('No keys yet.'));
              }
              return Column(children: [for (final key in found) _keyTile(key)]);
            },
          ),
          const Divider(height: 32),
          Text('Pair a program by PIN', style: text.titleSmall),
          const Text(
            'For a program with a key of its own, or the Canoa browser extension on this computer, which you give the PIN. A web page cannot pair by PIN.',
          ),
          const SizedBox(height: 8),
          _pinPanel(text),
          if (message != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(message!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
        ],
      ),
    );
  }

  Widget _entryTile(HostEntry entry) {
    String? url;
    String? provider;
    for (final listening in widget.supervisor.served) {
      if (listening.name != entry.name) continue;
      url = listening.url;
      provider = listening.harness ?? (listening.local ? listening.name.toLowerCase() : null);
    }
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: entry.kind == 'agent' ? SourceMark.provider(provider) : const Icon(Icons.extension_outlined),
      title: Text(entry.name),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SettingNote(
            config: config,
            configKey: ['host', entry.kind == 'agent' ? 'agents' : 'providers', entry.name],
            // Hosting writes [host] on its own, so a reset goes the same way.
            onReset: busy
                ? () {}
                : () => _run(() => _save(config.resetKey(['host', entry.kind == 'agent' ? 'agents' : 'providers', entry.name]))),
          ),
          Text(
            [entry.runs, ...entry.args].join(' ') + (url == null ? '' : '\n$url'),
            maxLines: 3,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (url != null)
            IconButton(
              tooltip: 'Copy its address',
              icon: const Icon(Icons.copy),
              onPressed: () => Clipboard.setData(ClipboardData(text: url!)),
            ),
          IconButton(
            tooltip: 'Stop hosting ${entry.name}',
            icon: const Icon(Icons.delete_outline),
            onPressed: busy ? null : () => _remove(entry),
          ),
        ],
      ),
    );
  }

  Widget _keyTile(HostKey key) => ListTile(
    contentPadding: EdgeInsets.zero,
    leading: Icon(key.accepted ? Icons.verified_user_outlined : Icons.hourglass_top),
    title: Text(key.accepted ? 'Paired' : 'Waiting${key.from == null ? '' : ' (from ${key.from})'}'),
    subtitle: _Id(key.id),
    trailing: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (!key.accepted) TextButton(onPressed: busy ? null : () => _accept(key), child: const Text('Pair…')),
        IconButton(
          tooltip: key.accepted ? 'Remove this key' : 'Turn this key away',
          icon: const Icon(Icons.remove_circle_outline),
          onPressed: busy ? null : () => _revoke(key),
        ),
      ],
    ),
  );

  Widget _pinPanel(TextTheme text) {
    final running = pairing;
    if (running != null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SelectableText(
            running.pin,
            style: text.displaySmall?.copyWith(fontFamily: CanoaLook.monospaceFont, letterSpacing: 6),
          ),
          SelectableText(running.native ? 'Type it in the Canoa extension, in your browser' : 'Dial ${running.url}', style: text.bodySmall),
          const SizedBox(height: 4),
          const Text('The other side shows this host as:'),
          Row(
            children: [
              Expanded(child: _Id(running.hostId)),
              ?_method(running.method),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)),
              const SizedBox(width: 8),
              const Expanded(child: Text('Waiting for the PIN. One wrong try ends it.')),
              TextButton(onPressed: _cancelPin, child: const Text('Cancel')),
            ],
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (paired != null) ...[const Text('Paired:'), _Id(paired!), const SizedBox(height: 8)],
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            OutlinedButton.icon(
              onPressed: busy || !widget.supervisor.managesKeys ? null : _showPin,
              icon: const Icon(Icons.pin_outlined),
              label: const Text('Show a PIN'),
            ),
            OutlinedButton.icon(
              onPressed: busy || !widget.supervisor.managesKeys ? null : () => _showPin(browser: true),
              icon: const Icon(Icons.extension_outlined),
              label: const Text('Pair the browser extension'),
            ),
          ],
        ),
      ],
    );
  }
}

/// The proof a host runs, shown beside its id, or nothing from a host that does not say.
Widget? _method(String? method) => method == null ? null : Text(' ($method)');

/// A peer id whole, as text that can be selected and compared.
class _Id extends StatelessWidget {
  const _Id(this.id);
  final String id;
  @override
  Widget build(BuildContext context) =>
      SelectableText(id, style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12));
}

class _Status extends StatelessWidget {
  const _Status({required this.supervisor});
  final HostSupervisor supervisor;

  @override
  Widget build(BuildContext context) {
    final (icon, said) = switch (supervisor.phase) {
      HostPhase.absent => (Icons.block, 'This build carries no host, so nothing is hosted.'),
      HostPhase.off => (Icons.power_settings_new, 'Left off, by canoa.cfg\'s local_host or --no-local-host.'),
      HostPhase.idle => (Icons.pause_circle_outline, 'Nothing to host: add an agent.'),
      HostPhase.starting => (Icons.hourglass_top, 'Starting…'),
      HostPhase.running => (Icons.check_circle_outline, supervisor.runningSaid),
      HostPhase.waiting => (Icons.restart_alt, 'It stopped: starting again soon. ${supervisor.problem ?? ''}'),
      HostPhase.failed => (Icons.error_outline, 'canoa.cfg is not right for it: ${supervisor.problem ?? ''}'),
      HostPhase.held => (Icons.people_outline, supervisor.problem ?? 'Another host runs on this home.'),
      HostPhase.stopped => (Icons.stop_circle_outlined, 'Stopped.'),
    };
    final peer = supervisor.peerId;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Icon(icon),
            const SizedBox(width: 8),
            Expanded(child: Text(said)),
            if (supervisor.phase == HostPhase.failed ||
                supervisor.phase == HostPhase.waiting ||
                supervisor.phase == HostPhase.held)
              TextButton(onPressed: () => unawaited(supervisor.restart()), child: const Text('Try again')),
          ],
        ),
        if (peer != null) ...[
          const SizedBox(height: 8),
          const Text('A browser pins this host as:'),
          Row(
            children: [
              Expanded(child: _Id(peer)),
              ?_method(supervisor.peerMethod),
              IconButton(
                tooltip: 'Copy the host id',
                icon: const Icon(Icons.copy),
                onPressed: () => Clipboard.setData(ClipboardData(text: peer)),
              ),
            ],
          ),
        ],
      ],
    );
  }
}

class _AddAgentDialog extends StatefulWidget {
  const _AddAgentDialog();
  @override
  State<_AddAgentDialog> createState() => _AddAgentDialogState();
}

/// The dialog is drawn from what the host says an agent entry takes (`providers/list`), so it offers every key the host reads and
/// nothing of the application's own; what it gives back is the keys the person set, by the names canoa.cfg gives them.
class _AddAgentDialogState extends State<_AddAgentDialog> {
  Map<String, Object?> values = {};
  String? error;

  /// What a choice the host knows the values of offers, with the first entry for leaving it unset.
  static List<SchemaChoice> _choices(String none, Iterable<SchemaChoice> offered) => [SchemaChoice(value: '', label: none), ...offered];

  void _submit(EntryForm form) {
    final kept = collectSchemaValues(form.schema, form.form, values);
    final roots = kept['working_roots'];
    final problem =
        schemaProblem(form.schema, form.form, values) ??
        (kept['program'] == null && kept['harness'] == null
            ? 'Name a program or a harness.'
            : roots is List && roots.any((folder) => !isWholeFolderPath('$folder'))
            ? 'The workspace must be a whole path.'
            : null);
    if (problem != null) return setState(() => error = problem);
    Navigator.pop(context, kept);
  }

  @override
  Widget build(BuildContext context) {
    final form = providerCatalog.agentEntry;
    return AlertDialog(
      title: const Text('Add an agent'),
      content: SizedBox(
        width: 560,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Padding(
                padding: EdgeInsets.symmetric(horizontal: 16),
                child: Text('It runs as you, for each browser that connects.'),
              ),
              if (form == null)
                const Padding(
                  key: Key('agent-entry-unknown'),
                  padding: EdgeInsets.all(16),
                  child: Text('This computer has not yet said what an agent takes.'),
                )
              else
                SchemaFields(
                  schema: form.schema,
                  form: form.form,
                  values: values,
                  idPrefix: 'agent',
                  keys: const {'usage': 'usage-reader'},
                  choices: {
                    // What the host offers is what can be chosen: its bundled agents' readers and the ones its libraries and sources add.
                    'usage_reader': _choices('As its command says', [
                      for (final reader in providerCatalog.usageReaders)
                        SchemaChoice(
                          value: reader.id,
                          label: reader.origin.isEmpty || reader.origin == 'bundled' ? reader.id : '${reader.id} (${reader.origin})',
                        ),
                    ]),
                    'harness': _choices('None: it runs a program', [
                      for (final each in providerCatalog.providers)
                        if (each.harness) SchemaChoice(value: each.id, label: each.title, detail: each.id),
                    ]),
                  },
                  onValues: (held) => values = held,
                ),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 12, left: 16, right: 16),
                  child: Text(error!, key: const Key('agent-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(onPressed: form == null ? null : () => _submit(form), child: const Text('Add')),
      ],
    );
  }
}
