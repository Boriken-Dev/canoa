import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'aos.dart';
import 'overlay_layer.dart';
import 'overlays.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'profile_layers.dart';
import 'project_trust.dart';

/// The effective profile of each agent, every value beside the layer it came from; the overlays, which install and sync
/// only on a click that shows what changes; and the trusted project directories, each revocable.
class ProfilesDialog extends StatefulWidget {
  const ProfilesDialog({super.key, required this.config, required this.agents, this.home, this.workingDirectory,
      this.onPin});

  final CanoaConfig config;

  /// The agents to choose among, by label.
  final List<String> agents;

  /// This computer's overlays and trust; none in a web build.
  final ProfileHome? home;

  /// The directory whose project profile is shown first, if any.
  final String? workingDirectory;

  /// Pins the overlay [name] in Canoa.cfg to the tree digest just installed, and gives the configuration saved.
  final Future<CanoaConfig?> Function(String name, String digest)? onPin;

  @override
  State<ProfilesDialog> createState() => ProfilesDialogState();
}

class ProfilesDialogState extends State<ProfilesDialog> {
  late String agent = widget.agents.isEmpty ? '' : widget.agents.first;
  late CanoaConfig config = widget.config;
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
      overlays: config.overlays,
      user: config.profile,
      agents: config.agentProfiles,
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
    if (apply != true) return;
    final record = await home.overlays.apply(update);
    final pinned = await widget.onPin?.call(name, record.digest!);
    if (pinned != null && mounted) setState(() => config = pinned);
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
            Text(visible('${note.layer}: ${note.text}'), style: Theme.of(context).textTheme.bodySmall),
          const SizedBox(height: 12),
          Text('Overlays', style: Theme.of(context).textTheme.titleMedium),
          if (config.overlays.isEmpty) const Text('None: add one to [overlays] in Canoa.cfg.'),
          for (final MapEntry(key: name, value: source) in config.overlays.entries) _overlayRow(name, source),
          for (final report in found?.effective.overlays ?? const <OverlayReport>[]) OverlayReportView(report: report),
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
          if (problem != null) Text(visible(problem!), style: TextStyle(color: Theme.of(context).colorScheme.error)),
        ],
      ))),
      actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
    );
  }

  AosLayer? _aos(String name) => layers?.layers.where((layer) => layer.aos?.identity == name).firstOrNull?.aos;

  /// The overlay's `[about]` summary, shown as plain text and never given to an agent.
  String? _summary(String name) => _aos(name)?.about['summary'];

  Widget? _icon(String name) {
    final aos = _aos(name);
    final path = aos?.about['icon'];
    final bytes = path == null ? null : aos?.icon[path];
    return path == null || bytes == null ? null : OverlayIcon(path: path, bytes: bytes);
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
      case DirOverlay(:final dir, :final pin):
        return ListTile(dense: true, leading: _icon(name), title: Text(name), subtitle: Text([
          ?_summary(name),
          'The directory $dir, read in place, ${pin == null ? 'not pinned' : 'pinned to $pin'}',
        ].join('\n')));
      case GitOverlay():
        final installed = home?.overlays.installed(name);
        final state = home == null
            ? 'Installed by the desktop app'
            : installed == null
            ? 'Not installed'
            : installed.source != source.spec
            ? 'Installed from ${installed.source}: sync to use ${source.spec}'
            : 'At ${installed.commit}, ${source.pin == null ? 'not pinned' : 'pinned to ${source.pin}'}';
        return ListTile(
          dense: true,
          leading: _icon(name),
          title: Text('$name  ${source.spec}'),
          subtitle: Text(visible([?_summary(name), state].join('\n'))),
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
          Text('${update.source.spec}\n${update.from == null ? '' : 'From ${update.from}\n'}To ${update.commit}\n'
              'Tree digest ${update.digest}: applying pins it in Canoa.cfg, and Canoa checks it each time the overlay is used.'),
          if (update.changesNothing) const Text('Nothing changes.'),
          files('Added', update.added),
          files('Changed', update.changed),
          files('Removed', update.removed),
          files('Left out: no part of an overlay (.git, node_modules, ...), never installed', update.skipped),
          if (update.manifest case final manifest?) ...[
            const Text('Its overlay.cfg, which reaches agents once applied:'),
            SelectableText(visible(manifest), style: const TextStyle(fontFamily: 'monospace')),
          ] else
            const Text('It has no overlay.cfg: its conventional files (skills, CONTEXT.md, env, rules) apply.'),
          const Text('Nothing from it runs now, and Canoa runs no setup or env script of it. What it sets reaches agents '
              'started after this; Settings, Profiles shows what Canoa does not apply.'),
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
              EnvReference(:final reference) => '$key from $reference',
              _ => '$key = ${describe(value)}',
            },
        ].join(', ')}';

  static String describe(Object? value) => switch (value) {
    EnvLiteral(:final value) => value,
    EnvFromEnv(:final name) => 'from the environment variable $name',
    EnvFromFile(:final path) => 'from the file $path',
    EnvText(:final text) => text,
    EnvPath(:final path) => "the overlay's $path",
    EnvReference(:final reference, :final optional) => 'from $reference${optional ? ', if granted' : ''}',
    EnvPaths(:final paths) =>
      [for (final (path, append) in paths) "${append ? 'appends' : 'prepends'} the overlay's $path"].join(', '),
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
      for (final value in effective.contexts) ('context', describe(value.value), '${value.layer}'),
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
            Padding(padding: const EdgeInsets.all(4), child: Text(visible(value))),
            Padding(padding: const EdgeInsets.all(4), child: Text(layer, key: Key('profile-layer-$key'))),
          ]),
      ],
    );
  }
}

/// An AOS overlay's report: its digest, what Canoa does not apply yet, what is not granted, and its weakenings.
class OverlayReportView extends StatelessWidget {
  const OverlayReportView({super.key, required this.report});
  final OverlayReport report;

  @override
  Widget build(BuildContext context) {
    final lines = [
      '${report.layer}: digest ${report.digest}',
      for (final item in report.notApplied) '  $item: not applied by Canoa yet',
      for (final item in report.ungranted) '  $item: none of your layers shares it, so it is left unset',
      for (final item in report.unresolved) '  $item: none of your layers shares it, so no session can start',
      for (final weakening in report.weakenings)
        '  weakening ${weakening.id}${weakening.subject.isEmpty ? '' : ': ${weakening.subject}'}',
    ];
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Text(visible(lines.join('\n')), key: Key('overlay-report-${report.layer.name}'),
          style: Theme.of(context).textTheme.bodySmall),
    );
  }
}

/// An overlay's `[about]` icon, from the bytes its tree digest covered: PNG or WebP only, its size and the dimensions in
/// its header checked against the specification's limits before it is decoded, shown as pixels only; anything refused or
/// undecodable shows a placeholder, never another image.
class OverlayIcon extends StatelessWidget {
  const OverlayIcon({super.key, required this.path, required this.bytes});
  final String path;
  final List<int> bytes;

  static const placeholder = Icon(Icons.extension, size: 32);

  @override
  Widget build(BuildContext context) {
    try {
      final header = checkIcon(path, bytes);
      return Image.memory(
        Uint8List.fromList(bytes),
        width: 32,
        height: 32,
        fit: BoxFit.contain,
        cacheWidth: header.width > 64 ? 64 : null,
        gaplessPlayback: true,
        errorBuilder: (_, _, _) => placeholder,
      );
    } on AosException {
      return placeholder;
    }
  }
}