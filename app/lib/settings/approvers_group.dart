import 'dart:convert';

import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../permissions/approvers.dart';
import '../shell/confirm_remove.dart';
import 'settings_controller.dart';
import 'settings_row.dart';

/// The approvers of Settings > Permissions: where else a permission request can be answered from besides this window. Nothing
/// answers for the person unless an entry is added here, and a request nobody answers in time is refused.
class ApproversGroup extends StatelessWidget {
  const ApproversGroup({super.key, required this.controller});

  final SettingsController controller;

  static const _timeouts = [60, 300, 900, 3600];

  /// The timeouts offered, 0 standing for none: a dropdown cannot choose null.
  List<(int, String)> _choices(BuildContext context, int? current) => [
    (0, context.t('approvers.timeout.none')),
    for (final seconds in {..._timeouts, ?current}.toList()..sort()) (seconds, context.t('approvers.timeout.seconds', {'count': seconds})),
  ];

  void _set(Approvers Function(Approvers) edit) => controller.change((c) => c.copyWith(approvers: edit(c.approvers)));

  void _replace(ApproverEntry entry) => _set(
    (held) => Approvers(
      entries: [for (final each in held.entries) each.name == entry.name ? entry : each],
      windowTimeoutSeconds: held.windowTimeoutSeconds,
    ),
  );

  Future<void> _remove(BuildContext context, ApproverEntry entry) async {
    final removing = await confirmRemoval(
      context,
      title: context.t('approvers.removing', {'name': entry.name}),
      body: context.t('approvers.removing_body'),
      confirmLabel: context.t('approvers.remove'),
      cancelLabel: context.t('approvers.keep'),
    );
    if (!removing) return;
    _set((held) => Approvers(entries: [for (final each in held.entries) if (each.name != entry.name) each], windowTimeoutSeconds: held.windowTimeoutSeconds));
  }

  Future<void> _add(BuildContext context) async {
    final taken = {for (final each in controller.config.approvers.entries) each.name};
    final made = await showDialog<ApproverEntry>(context: context, builder: (_) => _AddDialog(taken: taken));
    if (made == null) return;
    _set((held) => Approvers(entries: [...held.entries, made], windowTimeoutSeconds: held.windowTimeoutSeconds));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final approvers = controller.config.approvers;
      return SettingsGroup(
        title: context.t('approvers.title'),
        trailing: TextButton(key: const Key('approver-add'), onPressed: () => _add(context), child: Text(context.t('approvers.add'))),
        children: [
          SettingsRow(
            key: const Key('approvers-rule'),
            title: context.t('approvers.rule'),
            description: approvers.entries.isEmpty ? context.t('approvers.none') : null,
          ),
          for (final entry in approvers.entries) ...[
            SettingsRow(
              key: Key('approver-${entry.name}'),
              title: entry.name,
              description: entry.provider,
              control: TextButton(
                key: Key('approver-remove-${entry.name}'),
                onPressed: () => _remove(context, entry),
                child: Text(context.t('approvers.remove'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
            ),
            ChoiceRow<String>(
              title: context.t('approvers.detail'),
              description: context.t('approvers.detail_help'),
              child: true,
              fieldKey: Key('approver-detail-${entry.name}'),
              value: entry.detail,
              choices: [('tool', context.t('approvers.detail.tool')), ('command', context.t('approvers.detail.command'))],
              onChanged: (detail) => _replace(entry.copyWith(detail: detail)),
            ),
            ChoiceRow<int>(
              title: context.t('approvers.timeout'),
              description: context.t('approvers.timeout_help'),
              child: true,
              fieldKey: Key('approver-timeout-${entry.name}'),
              value: entry.timeoutSeconds ?? 0,
              choices: _choices(context, entry.timeoutSeconds),
              onChanged: (seconds) => _replace(seconds == 0 ? entry.copyWith(clearTimeout: true) : entry.copyWith(timeoutSeconds: seconds)),
            ),
            SwitchRow(
              key: Key('approver-away-${entry.name}'),
              title: context.t('approvers.away'),
              description: context.t('approvers.away_help'),
              child: true,
              value: entry.onlyWhenAway,
              onChanged: (away) => _replace(entry.copyWith(onlyWhenAway: away)),
            ),
          ],
          ChoiceRow<int>(
            title: context.t('approvers.window_timeout'),
            description: context.t('approvers.window_timeout_help'),
            fieldKey: const Key('approver-window-timeout'),
            value: approvers.windowTimeoutSeconds ?? 0,
            choices: _choices(context, approvers.windowTimeoutSeconds),
            onChanged: (seconds) => _set((held) => Approvers(entries: held.entries, windowTimeoutSeconds: seconds == 0 ? null : seconds)),
          ),
        ],
      );
    },
  );
}

class _AddDialog extends StatefulWidget {
  const _AddDialog({required this.taken});

  final Set<String> taken;

  @override
  State<_AddDialog> createState() => _AddDialogState();
}

class _AddDialogState extends State<_AddDialog> {
  final _name = TextEditingController();
  final _provider = TextEditingController(text: 'policy');
  final _settings = TextEditingController(text: '{"rules": []}');
  String? _problem;

  @override
  void dispose() {
    _name.dispose();
    _provider.dispose();
    _settings.dispose();
    super.dispose();
  }

  void _save() {
    final name = _name.text.trim();
    if (!approverNameOk(name) || widget.taken.contains(name)) {
      setState(() => _problem = context.t('approvers.problem.name'));
      return;
    }
    final provider = _provider.text.trim();
    if (provider.isEmpty) {
      setState(() => _problem = context.t('approvers.problem.provider'));
      return;
    }
    Map<String, Object?> settings;
    try {
      final decoded = _settings.text.trim().isEmpty ? <String, Object?>{} : jsonDecode(_settings.text);
      if (decoded is! Map) throw const FormatException('not a table');
      settings = {for (final e in decoded.entries) '${e.key}': e.value};
    } on FormatException {
      setState(() => _problem = context.t('approvers.problem.settings'));
      return;
    }
    Navigator.pop(context, ApproverEntry(name: name, provider: provider, settings: settings));
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(context.t('approvers.add')),
    content: SizedBox(
      width: 420,
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        TextField(
          key: const Key('approver-name'),
          controller: _name,
          decoration: InputDecoration(labelText: context.t('approvers.name'), hintText: context.t('approvers.name_hint')),
        ),
        TextField(
          key: const Key('approver-provider'),
          controller: _provider,
          decoration: InputDecoration(labelText: context.t('approvers.provider')),
        ),
        TextField(
          key: const Key('approver-settings'),
          controller: _settings,
          minLines: 3,
          maxLines: 8,
          style: const TextStyle(fontFamily: 'monospace'),
          decoration: InputDecoration(labelText: context.t('approvers.settings')),
        ),
        if (_problem case final problem?) Padding(padding: const EdgeInsets.only(top: 8), child: Text(problem, key: const Key('approver-problem'))),
      ]),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: Text(context.t('approvers.cancel'))),
      FilledButton(key: const Key('approver-save'), onPressed: _save, child: Text(context.t('approvers.save'))),
    ],
  );
}
