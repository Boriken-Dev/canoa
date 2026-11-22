import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../desktop/control_client.dart' show hostWords;
import 'entry_details.dart';
import 'overlay_page.dart' show oneLine;
import 'programs.dart';
import 'programs_view.dart';
import 'runtime_mark.dart';
import 'store_runtimes.dart';

/// One runtime's page, laid out as an overlay's: the runtime's icon and name, what its store says it is, its version installed and
/// the newest, where it is installed, where it downloads from and the checksum it is checked against, what uses it, and Install,
/// Update, Roll back and Remove, each through the host's plan that the person confirms first. Nothing is fetched until a button is
/// pressed.
class RuntimePage extends StatelessWidget {
  const RuntimePage({super.key, required this.programs, required this.provider, this.config});

  final ProgramStores programs;

  /// The store's id, which the host's `store/*` methods name.
  final String provider;

  /// canoa.cfg, for what uses the runtime; null leaves that out.
  final CanoaConfig? config;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: programs,
    builder: (context, _) {
      final item = RuntimeItem.find(programs, provider);
      if (item == null) {
        return Scaffold(
          appBar: AppBar(title: const Text('Runtime')),
          body: const Padding(padding: EdgeInsets.all(16), child: Text('The host does not list this runtime now.', key: Key('runtime-gone'))),
        );
      }
      final status = item.status;
      final copy = status.copy;
      final others = [for (final each in status.copies) if (each != copy) each.path];
      final commands = status.info.commands;
      final users = config == null ? const <RuntimeUser>[] : item.users(config!);
      return EntryDetails(
        key: Key('runtime-details-$provider'),
        title: item.name,
        leading: RuntimeMark(icon: item.icon, size: 56),
        from: 'its store',
        summary: item.about,
        state: !item.installed ? null : item.updateAvailable ? 'Update available' : 'Installed',
        overview: {
          if (item.by.isNotEmpty) 'Publisher': item.by,
          'Homepage': ?item.homepage,
        },
        source: {
          'Installed version': item.installed
              ? '${item.version ?? 'not read'}${item.managed ? '' : ' (found on this computer, not installed by Canoa)'}'
              : 'none',
          'Newest version': _newest(status),
          if (copy != null) 'Installed at': copy.path,
          if (others.isNotEmpty) 'Also found at': others.join(', '),
          if (copy != null && !copy.managed && copy.manager.isNotEmpty && copy.manager != 'UNKNOWN') 'Put there by': copy.manager.toLowerCase(),
          if (item.endpoints.isNotEmpty) 'Downloads from': item.endpoints.join(', '),
        },
        contents: false,
        sections: [
          DetailSection(key: const Key('runtime-uses'), title: 'What uses it', children: [
            if (commands.isEmpty)
              const Text('Nothing finds it on the PATH: the host starts it itself.', key: Key('runtime-uses-none'))
            else ...[
              DetailField(label: 'Commands it provides', value: commands.join(', ')),
              const Padding(
                padding: EdgeInsets.only(top: 4),
                child: Text('Once it is installed its folder is on the PATH of every agent, MCP server and tool the host starts, after your own entries.'),
              ),
              if (config != null)
                if (users.isEmpty)
                  const Text('Nothing in canoa.cfg starts one of them by name.', key: Key('runtime-uses-none'))
                else
                  for (final user in users)
                    Padding(
                      key: Key('runtime-user-${user.kind}-${user.name}'),
                      padding: const EdgeInsets.only(top: 4),
                      child: SelectableText(oneLine('${user.kind} ${user.name} starts ${user.command}')),
                    ),
            ],
          ]),
          _DownloadFacts(key: Key('runtime-download-$provider'), programs: programs, item: item),
        ],
        action: Wrap(spacing: 8, runSpacing: 8, children: [
          if (!item.managed)
            FilledButton(
              key: const Key('runtime-install'),
              onPressed: () => unawaited(runProgramAction(context, programs, status, 'install')),
              child: Text(item.installed ? 'Install a copy for Canoa' : 'Install'),
            ),
          if (item.managed && item.updateAvailable)
            FilledButton(
              key: const Key('runtime-update'),
              onPressed: () => unawaited(runProgramAction(context, programs, status, 'update')),
              child: const Text('Update'),
            ),
          if (item.managed)
            OutlinedButton(
              key: const Key('runtime-rollback'),
              onPressed: () => unawaited(rollBack(context, programs, status)),
              child: const Text('Roll back'),
            ),
          if (item.managed)
            OutlinedButton(
              key: const Key('runtime-remove'),
              onPressed: () => unawaited(runProgramAction(context, programs, status, 'uninstall')),
              child: const Text('Remove…'),
            ),
          TextButton(
            key: const Key('runtime-check'),
            onPressed: programs.loading ? null : () => unawaited(programs.refresh(latest: true, force: true)),
            child: const Text('Check for updates'),
          ),
        ]),
      );
    },
  );

  static String _newest(ProgramStatus status) {
    final latest = status.latest;
    if (latest == null) return 'not asked yet';
    if (!latest.known) return 'unknown${latest.unknownBecause == null ? '' : ': ${latest.unknownBecause}'}';
    return '${latest.version ?? 'unknown'}${status.updateAvailable ? ' (an update)' : ''}${latest.note == null ? '' : ' · ${latest.note}'}';
  }
}

/// The file an install would fetch and the checksum it is checked against, which the host asks the publisher for when the person
/// asks: the plan it makes is shown and nothing is installed.
class _DownloadFacts extends StatefulWidget {
  const _DownloadFacts({super.key, required this.programs, required this.item});

  final ProgramStores programs;
  final RuntimeItem item;

  @override
  State<_DownloadFacts> createState() => _DownloadFactsState();
}

class _DownloadFactsState extends State<_DownloadFacts> {
  ProgramPlan? _plan;
  String? _problem;
  bool _asking = false;

  Future<void> _look() async {
    setState(() {
      _asking = true;
      _problem = null;
    });
    try {
      final plan = await widget.item.plan();
      if (mounted) setState(() => _plan = plan);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = hostWords(error));
    } finally {
      if (mounted) setState(() => _asking = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final plan = _plan;
    return DetailSection(title: 'Download and checksum', children: [
      const Padding(
        padding: EdgeInsets.only(top: 2),
        child: Text('The host asks the publisher which file it would fetch and what to check it against. Nothing is installed.'),
      ),
      if (plan == null)
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(key: const Key('runtime-lookup'), onPressed: _asking ? null : () => unawaited(_look()), child: const Text('Look it up')),
        ),
      if (_asking) const Padding(padding: EdgeInsets.only(top: 4), child: LinearProgressIndicator()),
      if (_problem case final problem?)
        Text(oneLine(problem), key: const Key('runtime-lookup-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
      if (plan != null) ...[
        if (plan.version != null) DetailField(label: 'Version', value: plan.version!),
        if (plan.downloads.isEmpty)
          const Text('Nothing to fetch: it is at the newest version.', key: Key('runtime-nothing-to-fetch'))
        else
          for (final (at, each) in plan.downloads.indexed) ...[
            DetailField(key: Key('runtime-file-$at'), label: 'File', value: each.url),
            if (each.bytes != null) DetailField(label: 'Size', value: sizeText(each.bytes!)),
            if (each.verified)
              DetailField(key: Key('runtime-checksum-$at'), label: 'Checked against', value: '${each.how}${each.digest == null ? '' : ' ${each.digest}'}')
            else
              DetailField(key: Key('runtime-unverified-$at'), label: 'Unverified', value: each.how.isEmpty ? 'the publisher gives nothing to check it against' : each.how),
          ],
      ],
    ]);
  }
}
