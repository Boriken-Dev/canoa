import 'dart:convert';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'overlays.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'profile_layers.dart';
import 'project_trust.dart';

/// The effective profile of each agent, every value beside the layer it came from; the overlays, which install and sync
/// only on a click that shows what changes; and the trusted project directories, each revocable.
class ProfilesDialog extends StatefulWidget {
  const ProfilesDialog({super.key, required this.config, required this.agents, this.home, this.workingDirectory});

  final CanoaConfig config;

  /// The agents to choose among, by label.
  final List<String> agents;

  /// This computer's overlays and trust; none in a web build.
  final ProfileHome? home;

  /// The directory whose project profile is shown first, if any.
  final String? workingDirectory;

  @override
  State<ProfilesDialog> createState() => ProfilesDialogState();
}

class ProfilesDialogState extends State<ProfilesDialog> {
  late String agent = widget.agents.isEmpty ? '' : widget.agents.first;
  late final directory = TextEditingController(text: widget.workingDirectory ?? '');
  ProfileLayers? layers;
  String? busy, problem;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    directory.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final dir = directory.text.trim();
    final found = await profileLayers(
      overlays: widget.config.overlays,
      user: widget.config.profile,
      agents: widget.config.agentProfiles,
      agent: agent,
      home: widget.home,
      workingDirectory: dir.isEmpty ? null : dir,
    );
    await widget.home?.trust.load();
    if (mounted) setState(() => layers = found);
  }

  Future<void> _run(String what, Future<void> Function() action) async {
    setState(() {
      busy = what;
      problem = null;
    });
    try {
      await action();
    } on Object catch (error) {
      if (mounted) setState(() => problem = '$what: $error');
    } finally {
      if (mounted) setState(() => busy = null);
    }
    await _load();
  }

  Future<void> _sync(String name, GitOverlay source) => _run('Checking $name', () async {
    final home = widget.home!;
    final update = await home.overlays.check(name, source);
    if (!mounted) return;
    final apply = await showDialog<bool>(context: context, builder: (_) => OverlayUpdateDialog(update: update));
    if (apply == true) await home.overlays.apply(update);
  });

  Future<void> _trust(ProjectLayer project) async {
    final bytes = project.profile;
    if (bytes == null) return;
    final yes = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Trust this directory?'),
        content: SizedBox(width: 520, child: SingleChildScrollView(child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('${project.directory}\n\nIts $projectProfileFile, which agents started there will use as it is now. '
                'If it changes, it is ignored until you trust it again.'),
            const SizedBox(height: 12),
            SelectableText(visible(utf8.decode(bytes, allowMalformed: true)),
                style: const TextStyle(fontFamily: 'monospace')),
          ],
        ))),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Trust')),
        ],
      ),
    );
    if (yes == true) await _run('Trusting', () => widget.home!.trust.trust(project.directory, bytes));
  }

  @override
  Widget build(BuildContext context) {
    final home = widget.home;
    final found = layers;
    final project = found?.project;
    return AlertDialog(
      title: const Text('Profiles'),
      content: SizedBox(width: 680, child: SingleChildScrollView(child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          const Text('What an agent starts with, layer by layer: built-in, overlays, yours, the project\'s once you '
              'trust it, the agent\'s entry, then the session. A later layer wins.'),
          Row(children: [
            const Text('Agent  '),
            DropdownButton<String>(
              key: const Key('profile-agent'),
              value: widget.agents.contains(agent) ? agent : null,
              items: [for (final label in widget.agents) DropdownMenuItem(value: label, child: Text(label))],
              onChanged: (value) {
                if (value == null) return;
                setState(() => agent = value);
                _load();
              },
            ),
          ]),
          if (home != null)
            TextField(
              key: const Key('profile-directory'),
              controller: directory,
              decoration: const InputDecoration(labelText: 'Working directory, for its project profile'),
              onSubmitted: (_) => _load(),
            ),
          if (project != null) _projectRow(project),
          const SizedBox(height: 12),
          Text('Effective profile', style: Theme.of(context).textTheme.titleMedium),
          if (found == null) const LinearProgressIndicator() else EffectiveTable(effective: found.effective),
          for (final note in found?.notes ?? const <LayerNote>[])
            Text('${note.layer}: ${note.text}', style: Theme.of(context).textTheme.bodySmall),
          const SizedBox(height: 12),
          Text('Overlays', style: Theme.of(context).textTheme.titleMedium),
          if (widget.config.overlays.isEmpty) const Text('None: add one to [overlays] in Canoa.cfg.'),
          for (final MapEntry(key: name, value: source) in widget.config.overlays.entries) _overlayRow(name, source),
          if (home != null && home.trust.projects.isNotEmpty) ...[
            const SizedBox(height: 12),
            Text('Trusted directories', style: Theme.of(context).textTheme.titleMedium),
            for (final trusted in home.trust.projects)
              ListTile(
                dense: true,
                title: Text(trusted.directory),
                trailing: TextButton(
                  key: Key('profile-revoke-${trusted.directory}'),
                  onPressed: busy != null ? null : () => _run('Revoking', () => home.trust.revoke(trusted.directory)),
                  child: const Text('Stop trusting'),
                ),
              ),
          ],
          if (busy != null) Text('$busy…'),
          if (problem != null) Text(problem!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ],
      ))),
      actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
    );
  }

  Widget _projectRow(ProjectLayer project) {
    final (text, action) = switch (project.state) {
      TrustState.none => ('This directory has no $projectProfileFile', null),
      TrustState.untrusted => ('Its $projectProfileFile is ignored: you have not trusted this directory', 'Trust'),
      TrustState.changed => ('Its $projectProfileFile changed since you trusted it, so it is ignored', 'Trust again'),
      TrustState.trusted => ('Its $projectProfileFile is used', null),
    };
    return ListTile(
      dense: true,
      title: Text(text),
      trailing: action == null
          ? null
          : TextButton(key: const Key('profile-trust'), onPressed: busy != null ? null : () => _trust(project),
              child: Text(action)),
    );
  }

  Widget _overlayRow(String name, OverlaySource source) {
    final home = widget.home;
    switch (source) {
      case DirOverlay(:final dir):
        return ListTile(dense: true, title: Text(name), subtitle: Text('The directory $dir, read in place'));
      case GitOverlay():
        final installed = home?.overlays.installed(name);
        final state = home == null
            ? 'Installed by the desktop app'
            : installed == null
            ? 'Not installed'
            : installed.source != source.spec
            ? 'Installed from ${installed.source}: sync to use ${source.spec}'
            : 'At ${installed.commit}';
        return ListTile(
          dense: true,
          title: Text('$name  ${source.spec}'),
          subtitle: Text(state),
          trailing: home == null
              ? null
              : TextButton(
                  key: Key('profile-sync-$name'),
                  onPressed: busy != null ? null : () => _sync(name, source),
                  child: Text(installed == null ? 'Install…' : 'Check for changes…'),
                ),
        );
    }
  }
}

/// [text] with every control, format or bidirectional character but a line break or tab shown as `<U+XXXX>`, so what
/// a person reads is what the file says.
String visible(String text) => text.replaceAllMapped(
  RegExp('[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u00ad\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]'),
  (match) => '<U+${match[0]!.codeUnitAt(0).toRadixString(16).toUpperCase().padLeft(4, '0')}>',
);

/// What installing or syncing an overlay changes, shown before anything is: the commits and every file.
class OverlayUpdateDialog extends StatelessWidget {
  const OverlayUpdateDialog({super.key, required this.update});
  final OverlayUpdate update;

  @override
  Widget build(BuildContext context) {
    Widget files(String label, List<String> paths) => paths.isEmpty
        ? const SizedBox.shrink()
        : Text('$label (${paths.length}):\n${paths.map((path) => '  $path').join('\n')}');
    return AlertDialog(
      title: Text(update.from == null ? 'Install ${update.name}?' : 'Sync ${update.name}?'),
      content: SizedBox(width: 520, child: SingleChildScrollView(child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('${update.source.spec}\n${update.from == null ? '' : 'From ${update.from}\n'}To ${update.commit}'),
          if (update.changesNothing) const Text('Nothing changes.'),
          files('Added', update.added),
          files('Changed', update.changed),
          files('Removed', update.removed),
          files('Left out: links and submodules are never installed', update.skipped),
          if (update.profile case final profile?) ...[
            const Text('Its profile.toml, which reaches agents once applied:'),
            SelectableText(visible(profile), style: const TextStyle(fontFamily: 'monospace')),
          ] else
            const Text('It has no profile.toml, so it sets nothing.'),
          const Text('Nothing from it runs now. Its profile reaches agents started after this.'),
        ],
      ))),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(
          key: const Key('overlay-apply'),
          onPressed: update.changesNothing ? null : () => Navigator.pop(context, true),
          child: Text(update.from == null ? 'Install' : 'Apply'),
        ),
      ],
    );
  }
}

/// The effective profile as rows: what, its value (a reference as written, never resolved), and its layer.
class EffectiveTable extends StatelessWidget {
  const EffectiveTable({super.key, required this.effective});
  final EffectiveProfile effective;

  /// Each of [values] with where it comes from: `NAME from $VAR`, `NAME from the file P`, `NAME = text`.
  static String _reads(String what, Map<String, EnvValue> values) => values.isEmpty
      ? ''
      : '; $what ${[
          for (final MapEntry(:key, :value) in values.entries)
            switch (value) {
              EnvFromEnv(:final name) => '$key from \$$name',
              EnvFromFile(:final path) => '$key from the file $path',
              EnvLiteral(:final value) => '$key = $value',
            },
        ].join(', ')}';

  static String describe(Object? value) => switch (value) {
    EnvLiteral(:final value) => value,
    EnvFromEnv(:final name) => 'from the environment variable $name',
    EnvFromFile(:final path) => 'from the file $path',
    ProfileContext(:final text?) => text.length > 80 ? '${text.substring(0, 80)}…' : text,
    ProfileContext(:final file?) => 'the file $file',
    Skill(:final path) => path,
    McpStdio(:final command, :final args, :final env) =>
      'runs $command${args.isEmpty ? '' : ' ${args.join(' ')}'}${_reads('environment', env)}',
    McpHttp(:final url, :final sse, :final headers) => '${sse ? 'SSE' : 'HTTP'} $url${_reads('headers', headers)}',
    _ => '$value',
  };

  @override
  Widget build(BuildContext context) {
    final rows = <(String, String, String)>[
      if (effective.model case final value?) ('model', value.value, '${value.layer}'),
      if (effective.effort case final value?) ('effort', value.value, '${value.layer}'),
      if (effective.mode case final value?) ('mode', value.value, '${value.layer}'),
      if (effective.context case final value?) ('context', describe(value.value), '${value.layer}'),
      for (final MapEntry(:key, :value) in effective.env.entries) ('env.$key', describe(value.value), '${value.layer}'),
      for (final MapEntry(:key, :value) in effective.skills.entries)
        ('skills.$key', describe(value.value), '${value.layer}'),
      for (final MapEntry(:key, :value) in effective.mcp.entries) ('mcp.$key', describe(value.value), '${value.layer}'),
      for (final removed in effective.removed) (removed.key, 'removed, was ${removed.was}', '${removed.layer}'),
      for (final refused in effective.refused)
        (refused.key, 'left out: reads \$${refused.reads}, which none of your layers shares', '${refused.layer}'),
      for (final MapEntry(:key, :value) in effective.shareEnv.entries)
        ('share_env.$key', 'overlays and projects may read it', '$value'),
    ];
    if (rows.isEmpty) return const Text('Nothing is set: the agent starts with its own defaults.');
    return Table(
      columnWidths: const {0: IntrinsicColumnWidth(), 1: FlexColumnWidth(), 2: IntrinsicColumnWidth()},
      children: [
        for (final (key, value, layer) in rows)
          TableRow(key: ValueKey('profile-row-$key'), children: [
            Padding(padding: const EdgeInsets.all(4), child: Text(key)),
            Padding(padding: const EdgeInsets.all(4), child: Text(value)),
            Padding(padding: const EdgeInsets.all(4), child: Text(layer, key: Key('profile-layer-$key'))),
          ]),
      ],
    );
  }
}
