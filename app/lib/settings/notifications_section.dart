import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../attention/notifications_host.dart';
import '../brand.dart';
import '../desktop/control_client.dart' show ControlFailed, hostWords;
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import '../shell/confirm_remove.dart';
import 'settings_row.dart';

/// Settings › Notifications: where each kind of news from the host goes, and the notifiers that can be told. The host decides which
/// events happen and builds what a notice says; this only chooses among what it lists, and writes the choice through it.
class NotificationsSection extends StatefulWidget {
  const NotificationsSection({super.key, this.controller, this.onAllowNotifications});

  /// What the host says of its events and notifiers; null for a page with no host.
  final NotificationsController? controller;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  @override
  State<NotificationsSection> createState() => _NotificationsSectionState();
}

class _NotificationsSectionState extends State<NotificationsSection> {
  NotificationsController? get _host => widget.controller;

  @override
  void initState() {
    super.initState();
    unawaited(_host?.refresh());
  }

  @override
  void didUpdateWidget(NotificationsSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) unawaited(_host?.refresh());
  }

  void _say(String message) => ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(message)));

  Future<void> _route(NotificationEvent event, String name, bool on) async {
    final host = _host;
    if (host == null) return;
    final names = [
      for (final each in event.notifiers) if (each != name) each,
      if (on) name,
    ];
    try {
      await host.route(event.id, names);
    } on Object catch (error) {
      if (mounted) _say(context.t('notifications.route_failed', {'why': hostWords(error)}));
    }
  }

  Future<void> _add() async {
    final host = _host;
    if (host == null) return;
    await showDialog<void>(context: context, builder: (_) => AddNotifierDialog(controller: host));
  }

  Future<void> _remove(NotifierRow row) async {
    final host = _host;
    if (host == null) return;
    if (!await confirmRemoval(
      context,
      title: context.t('notifications.remove_title', {'name': row.name}),
      body: context.t('notifications.remove_body'),
      confirmLabel: context.t('notifications.remove'),
      cancelLabel: context.t('common.cancel'),
      dialogKey: const Key('remove-notifier'),
      cancelKey: null,
      confirmKey: const Key('remove-notifier-confirm'),
    )) {
      return;
    }
    try {
      await host.remove(row.name);
    } on Object catch (error) {
      if (mounted) _say(context.t('notifications.remove_failed', {'name': row.name, 'why': hostWords(error)}));
    }
  }

  Future<void> _test(NotifierRow row) async {
    final result = await _host!.test(row.name);
    if (!mounted) return;
    _say(result.ok ? context.t('notifications.test_sent') : context.t('notifications.test_failed', {'why': result.why ?? ''}));
  }

  String _eventTitle(NotificationEvent event) => AppText.of(context).maybe('notifications.event.${event.id}') ?? event.title;

  String _label(NotifierRow? row, String name) => name == 'desktop' ? context.t('notifications.desktop') : (row?.title ?? name);

  @override
  Widget build(BuildContext context) {
    final host = _host;
    final muted = Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(padding: const EdgeInsets.fromLTRB(4, 0, 4, 16), child: Text(context.t('notifications.intro'), style: muted)),
      if (host == null)
        Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Text(context.t('notifications.no_host'), key: const Key('notifications-no-host'), style: muted),
        )
      else
        ListenableBuilder(listenable: host, builder: (context, _) => _hosted(context, host)),
      if (widget.onAllowNotifications != null)
        SettingsGroup(title: context.t('notifications.browser'), children: [
          SettingsRow(
            title: context.t('notifications.allow'),
            description: context.t('notifications.allow_description'),
            machine: true,
            control: TextButton(
              key: const Key('settings-allow-notifications'),
              onPressed: () async {
                final allowed = await widget.onAllowNotifications!();
                if (!context.mounted) return;
                _say(allowed ? context.t('notifications.allowed', {'brand': Brand.name}) : context.t('notifications.not_allowed'));
              },
              child: Text(context.t('notifications.allow_button')),
            ),
          ),
        ]),
    ]);
  }

  Widget _hosted(BuildContext context, NotificationsController host) {
    final scheme = Theme.of(context).colorScheme;
    final names = [for (final row in host.notifiers) row.name];
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      if (host.problem case final problem?)
        Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Text(problem, key: const Key('notifications-problem'), style: TextStyle(color: scheme.error)),
        ),
      SettingsGroup(
        key: const Key('notification-events'),
        title: context.t('notifications.events'),
        children: [
          if (host.events.isEmpty)
            SettingsRow(title: host.loading ? context.t('notifications.loading') : context.t('notifications.events_help')),
          for (final event in host.events) _eventRow(event, host, names),
        ],
      ),
      SettingsGroup(
        key: const Key('notification-notifiers'),
        title: context.t('notifications.notifiers'),
        trailing: TextButton.icon(
          key: const Key('add-notifier'),
          onPressed: host.loading || host.providers.isEmpty ? null : () => unawaited(_add()),
          icon: const Icon(Icons.add, size: 18),
          label: Text(context.t('notifications.add')),
        ),
        children: [
          if (host.notifiers.isEmpty) SettingsRow(title: context.t('notifications.loading'), description: context.t('notifications.notifiers_help')),
          for (final row in host.notifiers) _notifierRow(row, host),
        ],
      ),
    ]);
  }

  Widget _eventRow(NotificationEvent event, NotificationsController host, List<String> names) {
    return SettingsRow(
      key: Key('notification-event ${event.id}'),
      title: _eventTitle(event),
      control: event.notifiers.isEmpty ? Text(context.t('notifications.off'), key: Key('notification-off ${event.id}')) : null,
      below: Wrap(spacing: 8, runSpacing: 4, children: [
        for (final name in names)
          FilterChip(
            key: Key('notification-route ${event.id} $name'),
            label: Text(_label(host.named(name), name)),
            selected: event.notifiers.contains(name),
            onSelected: (on) => unawaited(_route(event, name, on)),
          ),
      ]),
    );
  }

  Widget _notifierRow(NotifierRow row, NotificationsController host) {
    final scheme = Theme.of(context).colorScheme;
    final result = host.testOf(row.name);
    final testing = host.isTesting(row.name);
    final description = row.name == 'desktop' ? context.t('notifications.desktop_description') : providerHelp(context, row.provider, row.help);
    final state = row.ready ? context.t('notifications.ready') : row.state;
    return SettingsRow(
      key: Key('notifier ${row.name}'),
      title: row.name == 'desktop' ? context.t('notifications.desktop') : row.name,
      leading: Icon(row.name == 'desktop' ? Icons.desktop_windows_outlined : Icons.notifications_outlined, size: 24),
      description: [
        if (row.name != 'desktop') providerTitle(context, row.provider, row.title),
        ?description,
        state,
        if (row.replies) context.t('notifications.replies'),
        if (row.origin != null && !row.removable && row.name != 'desktop') context.t('notifications.from_layer', {'layer': row.origin}),
      ].join(' · '),
      below: testing
          ? const LinearProgressIndicator(key: Key('notifier-testing'))
          : result == null
              ? null
              : Text(
                  result.ok ? context.t('notifications.test_sent') : context.t('notifications.test_failed', {'why': result.why ?? ''}),
                  key: Key('notifier-test-result ${row.name}'),
                  style: TextStyle(color: result.ok ? scheme.primary : scheme.error),
                ),
      control: Row(mainAxisSize: MainAxisSize.min, children: [
        OutlinedButton(
          key: Key('notifier-test ${row.name}'),
          onPressed: testing ? null : () => unawaited(_test(row)),
          child: Text(context.t('notifications.test')),
        ),
        if (row.removable)
          IconButton(
            key: Key('notifier-remove ${row.name}'),
            tooltip: context.t('notifications.remove'),
            icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
            onPressed: testing ? null : () => unawaited(_remove(row)),
          ),
      ]),
    );
  }
}

/// The members of [schema] that are secrets: the host keeps them and never writes them to canoa.cfg.
Set<String> _secretKeys(Map<String, Object?> schema) => {
  for (final MapEntry(:key, :value) in schemaProperties(schema).entries)
    if (value is Map && (value['writeOnly'] == true || value['x-sensitive'] == true)) key,
};

/// Adds one notifier from a provider the host offers: its name, the kind and the form that kind's schema describes.
class AddNotifierDialog extends StatefulWidget {
  const AddNotifierDialog({super.key, required this.controller});

  final NotificationsController controller;

  @override
  State<AddNotifierDialog> createState() => _AddNotifierDialogState();
}

class _AddNotifierDialogState extends State<AddNotifierDialog> {
  final _name = TextEditingController();
  final _values = <String, Map<String, Object?>>{};
  String? _providerId;
  int _generation = 0;
  String? _problem;
  bool _saving = false;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  NotifierProvider? get _provider {
    final all = widget.controller.providers;
    return all.where((each) => each.id == _providerId).firstOrNull ?? all.firstOrNull;
  }

  Future<void> _submit() async {
    final name = _name.text.trim();
    if (name.isEmpty) return setState(() => _problem = context.t('notifications.name_missing'));
    if (!RegExp(r'^[a-z0-9_-]{1,32}$').hasMatch(name)) return setState(() => _problem = context.t('notifications.name_bad'));
    if (name == 'desktop' || widget.controller.named(name) != null) return setState(() => _problem = context.t('notifications.name_taken'));
    final provider = _provider;
    if (provider == null) return setState(() => _problem = context.t('notifications.no_kind'));
    final schema = provider.schema ?? const <String, Object?>{};
    final form = provider.form ?? const <String, Object?>{};
    final typed = _values[provider.id] ?? const <String, Object?>{};
    final problem = schema.isEmpty ? null : schemaProblem(schema, form, typed);
    if (problem != null) return setState(() => _problem = problem);
    final all = schema.isEmpty ? typed : collectSchemaValues(schema, form, typed);
    final secrets = _secretKeys(schema);
    setState(() {
      _problem = null;
      _saving = true;
    });
    try {
      await widget.controller.save(
        name,
        provider.id,
        {for (final MapEntry(:key, :value) in all.entries) if (!secrets.contains(key)) key: value},
        {for (final MapEntry(:key, :value) in all.entries) if (secrets.contains(key) && value is String && value.isNotEmpty) key: value},
      );
      if (mounted) Navigator.of(context).pop();
    } on ControlFailed catch (error) {
      if (mounted) setState(() => _problem = error.message);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = '$error');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final providers = widget.controller.providers;
    final provider = _provider;
    final schema = provider?.schema ?? const <String, Object?>{};
    final form = provider?.form ?? const <String, Object?>{};
    return AlertDialog(
      key: const Key('add-notifier-dialog'),
      title: Text(context.t('notifications.add_title')),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            TextField(
              key: const Key('notifier-name'),
              controller: _name,
              decoration: InputDecoration(labelText: context.t('notifications.name'), helperText: context.t('notifications.name_help')),
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: 12),
            DropdownButtonFormField<String>(
              key: const Key('notifier-kind-picker'),
              isExpanded: true,
              initialValue: provider?.id,
              decoration: InputDecoration(labelText: context.t('notifications.kind')),
              items: [for (final each in providers) DropdownMenuItem<String>(value: each.id, child: Text(providerTitle(context, each.id, each.title)))],
              onChanged: _saving
                  ? null
                  : (id) => setState(() {
                        _providerId = id;
                        _generation++;
                        _problem = null;
                      }),
            ),
            if (providerHelp(context, provider?.id ?? '', provider?.help) case final help?)
              Padding(padding: const EdgeInsets.only(top: 6), child: Text(help, style: Theme.of(context).textTheme.bodySmall)),
            if (provider != null && schema.isNotEmpty) ...[
              const SizedBox(height: 8),
              SchemaFields(
                key: ValueKey('notifier-${provider.id}-$_generation'),
                schema: schema,
                form: form,
                values: _values[provider.id] ?? const {},
                idPrefix: 'notifier',
                textPrefix: 'notifications.provider.${provider.id}',
                unsetEntry: true,
                onValues: (values) => _values[provider.id] = values,
                onSubmitted: () => unawaited(_submit()),
              ),
            ],
            if (_problem != null) ...[
              const SizedBox(height: 10),
              Text(_problem!, key: const Key('add-notifier-problem'), style: Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error)),
            ],
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: _saving ? null : () => Navigator.of(context).pop(), child: Text(context.t('common.cancel'))),
        FilledButton(
          key: const Key('add-notifier-submit'),
          onPressed: _saving || provider == null ? null : _submit,
          child: Text(_saving ? context.t('notifications.adding') : context.t('notifications.add_submit')),
        ),
      ],
    );
  }
}

/// What a notifier provider is called: the app's own word for it when it has one, else the title the provider gives.
String providerTitle(BuildContext context, String id, String title) => AppText.of(context).maybe('notifications.provider.$id') ?? title;

/// What a notifier provider says of itself: the app's own words for it when it has them, else the help the provider gives.
String? providerHelp(BuildContext context, String id, String? help) => AppText.of(context).maybe('notifications.provider.$id.help') ?? help;
