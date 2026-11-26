import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import 'settings_row.dart';

/// What the app asks the host with: `telemetry/catalog`, `telemetry/status`, `telemetry/test` and `config/change`.
typedef TelemetryCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// The three groups of signals the person chooses between, in the order the page lists them.
const telemetryGroups = ['spans', 'metrics', 'logs'];

/// One attribute of a signal the host records.
final class TelemetryAttribute {
  const TelemetryAttribute({required this.name, required this.type, required this.optional, this.values = const []});

  factory TelemetryAttribute.read(Map<Object?, Object?> raw) => TelemetryAttribute(
    name: '${raw['name']}',
    type: '${raw['type']}',
    optional: raw['optional'] == true,
    values: [for (final each in raw['values'] as List<Object?>? ?? const []) '$each'],
  );

  final String name, type;
  final bool optional;

  /// The closed set of words an attribute of type `choice` holds.
  final List<String> values;

  /// What the attribute holds, in a few words the page can show beside its name.
  String get shape => switch (type) {
    'choice' => values.join(' | '),
    'int' => 'whole number',
    _ => 'short name',
  };
}

/// One signal in the host's list of what it records.
final class TelemetrySignal {
  const TelemetrySignal({required this.name, required this.group, required this.kind, required this.unit, required this.description, required this.attributes});

  factory TelemetrySignal.read(Map<Object?, Object?> raw) => TelemetrySignal(
    name: '${raw['name']}',
    group: '${raw['group']}',
    kind: '${raw['kind']}',
    unit: raw['unit'] is String ? raw['unit'] as String : '',
    description: (raw['description'] ?? '').toString(),
    attributes: [
      for (final each in raw['attributes'] as List<Object?>? ?? const [])
        if (each is Map<Object?, Object?>) TelemetryAttribute.read(each),
    ],
  );

  final String name, group, kind, unit, description;
  final List<TelemetryAttribute> attributes;
}

/// One exporter the person named, with how it is doing.
final class TelemetryExporter {
  const TelemetryExporter({required this.name, required this.provider, required this.state, required this.queued, required this.dropped, this.lastError});

  factory TelemetryExporter.read(Map<Object?, Object?> raw) => TelemetryExporter(
    name: '${raw['name']}',
    provider: '${raw['provider']}',
    state: '${raw['state']}',
    queued: raw['queued'] is int ? raw['queued'] as int : 0,
    dropped: raw['dropped'] is int ? raw['dropped'] as int : 0,
    lastError: raw['last_error'] is String ? raw['last_error'] as String : null,
  );

  final String name, provider, state;
  final int queued, dropped;
  final String? lastError;
}

/// A kind of exporter the host offers, with the form of its settings.
final class TelemetryProvider {
  const TelemetryProvider({required this.id, required this.title, required this.schema});

  factory TelemetryProvider.read(Map<Object?, Object?> raw) => TelemetryProvider(
    id: '${raw['id']}',
    title: (raw['title'] ?? raw['id']).toString(),
    schema: raw['schema'] is Map ? (raw['schema'] as Map).cast<String, Object?>() : const {},
  );

  final String id, title;
  final Map<String, Object?> schema;
}

/// Telemetry export: the list of what the host records and what it never does, whether export runs, and the exporters. Everything
/// here is the host's: the page draws what `telemetry/catalog` and `telemetry/status` answer and writes `[host.telemetry]` through
/// `config/change`. The host reads that table when it starts, so [restartPending] is true once something was written.
class TelemetrySettings extends ChangeNotifier {
  TelemetrySettings(this._call, {this.restart});

  final TelemetryCall _call;

  /// Has the host start again, which applies the change; null where this page cannot.
  final Future<bool> Function()? restart;

  bool _disposed = false, _loading = false, _loaded = false, _restartPending = false;
  String? _problem;
  String _digest = '', _acknowledged = '', _state = 'off', _textRule = '';
  String? _paused;
  bool _enabled = false;
  List<TelemetrySignal> _signals = const [];
  List<String> _never = const [], _record = telemetryGroups;
  List<TelemetryExporter> _exporters = const [];
  List<TelemetryProvider> _providers = const [];

  bool get loading => _loading;
  bool get loaded => _loaded;
  bool get restartPending => _restartPending;
  String? get problem => _problem;

  /// The digest of the list the host records now.
  String get digest => _digest;

  /// The digest of the list the person accepted, empty when none.
  String get acknowledged => _acknowledged;
  bool get enabled => _enabled;

  /// `off`, `paused` or `running`.
  String get state => _state;

  /// Why export is paused, in the host's words.
  String? get paused => _paused;

  /// Whether the person accepted the list the host records now.
  bool get accepted => _acknowledged.isNotEmpty && _acknowledged == _digest;

  /// Whether the person accepted a list, and the host's is another now.
  bool get changed => _acknowledged.isNotEmpty && _acknowledged != _digest;
  List<TelemetrySignal> get signals => _signals;
  List<String> get never => _never;
  String get textRule => _textRule;
  List<String> get record => _record;
  List<TelemetryExporter> get exporters => _exporters;
  List<TelemetryProvider> get providers => _providers;

  List<TelemetrySignal> signalsOf(String group) => [
    for (final signal in _signals)
      if (signal.group == group) signal,
  ];

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  static Map<Object?, Object?> _map(Object? value) => value is Map<Object?, Object?> ? value : const {};

  /// Reads the list and the state.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      final catalog = _map(await _call('telemetry/catalog', const {}));
      final status = _map(await _call('telemetry/status', const {}));
      _digest = '${catalog['digest']}';
      _textRule = catalog['text_rule'] is String ? catalog['text_rule'] as String : '';
      _signals = [
        for (final each in catalog['signals'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) TelemetrySignal.read(each),
      ];
      _never = [for (final each in catalog['never'] as List<Object?>? ?? const []) '$each'];
      _enabled = status['enabled'] == true;
      _state = '${status['state']}';
      _paused = status['paused'] is String ? status['paused'] as String : null;
      _acknowledged = status['acknowledged'] is String ? status['acknowledged'] as String : '';
      _record = [for (final each in status['record'] as List<Object?>? ?? telemetryGroups) '$each'];
      _exporters = [
        for (final each in status['exporters'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) TelemetryExporter.read(each),
      ];
      _providers = [
        for (final each in status['providers'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) TelemetryProvider.read(each),
      ];
      _problem = null;
      _loaded = true;
    } on ControlFailed catch (error) {
      _problem = error.code == -32001 ? 'only the person at this computer may see or change this' : error.message;
    } on Object catch (error) {
      _problem = '$error';
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  /// Makes `ops` in canoa.cfg. Returns what is wrong in the host's words, or null once it holds them.
  Future<String?> _write(List<Map<String, Object?>> ops) async {
    try {
      await _call('config/change', {'ops': ops});
      _restartPending = true;
      await load();
      return null;
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return '$error';
    }
  }

  /// The person has read the list and accepts it: `acknowledged` is its digest and export is on, in one write.
  Future<String?> accept() => _write([
    {'path': ['host', 'telemetry', 'acknowledged'], 'value': _digest},
    {'path': ['host', 'telemetry', 'enabled'], 'value': true},
  ]);

  /// Turns export off; what was accepted stays.
  Future<String?> turnOff() => _write([
    {'path': ['host', 'telemetry', 'enabled'], 'value': false},
  ]);

  /// Records [group] or not.
  Future<String?> setRecorded(String group, {required bool on}) {
    final next = {..._record};
    on ? next.add(group) : next.remove(group);
    return _write([
      {'path': ['host', 'telemetry', 'record'], 'value': [for (final each in telemetryGroups) if (next.contains(each)) each]},
    ]);
  }

  /// Names an exporter of [provider] with [settings].
  Future<String?> addExporter(String name, String provider, Map<String, Object?> settings) => _write([
    {
      'path': ['host', 'telemetry', 'exporters', name],
      'value': {'provider': provider, if (settings.isNotEmpty) 'settings': settings},
    },
  ]);

  /// Takes the exporter [name] out.
  Future<String?> removeExporter(String name) => _write([
    {'path': ['host', 'telemetry', 'exporters', name]},
  ]);

  /// Sends one test record through [name]: null when it was delivered, else why not.
  Future<String?> test(String name) async {
    try {
      final answer = _map(await _call('telemetry/test', {'name': name}));
      return answer['ok'] == true ? null : '${answer['why'] ?? 'the test failed'}';
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return '$error';
    }
  }

  /// Has the host start again, so what was written takes hold.
  Future<void> restartHost() async {
    if (restart == null || !await restart!()) return;
    _restartPending = false;
    notifyListeners();
    unawaited(load());
  }
}

/// Settings, Telemetry: what the host records and never records, the switch, the groups and the exporters.
class TelemetrySection extends StatefulWidget {
  const TelemetrySection({super.key, required this.settings});

  final TelemetrySettings settings;

  @override
  State<TelemetrySection> createState() => _TelemetrySectionState();
}

class _TelemetrySectionState extends State<TelemetrySection> {
  bool _reading = false, _read = false;
  String? _note;

  @override
  void initState() {
    super.initState();
    if (!widget.settings.loaded && !widget.settings.loading) unawaited(widget.settings.load());
  }

  void _say(String? problem) {
    if (problem != null && mounted) setState(() => _note = problem);
  }

  Future<void> _accept() async {
    setState(() => _note = null);
    _say(await widget.settings.accept());
    if (mounted) setState(() => _reading = false);
  }

  Future<void> _addExporter() async {
    final settings = widget.settings;
    final made = await showDialog<_NewExporter>(context: context, builder: (_) => _AddExporterDialog(providers: settings.providers));
    if (made == null || !mounted) return;
    _say(await settings.addExporter(made.name, made.provider, made.settings));
  }

  Future<void> _removeExporter(TelemetryExporter exporter) async {
    final text = AppText.of(context);
    final sure = await showDialog<bool>(
      context: context,
      builder: (dialog) => AlertDialog(
        title: Text(text('telemetry.remove_title', {'name': exporter.name})),
        content: Text(text('telemetry.remove_body')),
        actions: [
          TextButton(key: const Key('telemetry-remove-cancel'), onPressed: () => Navigator.pop(dialog, false), child: Text(text('common.cancel'))),
          FilledButton(key: const Key('telemetry-remove-confirm'), onPressed: () => Navigator.pop(dialog, true), child: Text(text('telemetry.remove'))),
        ],
      ),
    );
    if (sure == true && mounted) _say(await widget.settings.removeExporter(exporter.name));
  }

  Future<void> _test(TelemetryExporter exporter) async {
    final why = await widget.settings.test(exporter.name);
    if (!mounted) return;
    final text = AppText.of(context);
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(
      SnackBar(content: Text(why == null ? text('telemetry.test_ok', {'name': exporter.name}) : text('telemetry.test_failed', {'name': exporter.name, 'why': why}))),
    );
  }

  Widget _signalRow(BuildContext context, TelemetrySignal signal) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final heading = signal.unit.isEmpty ? signal.name : [signal.name, signal.unit].join('  ');
    return Padding(
      key: Key('telemetry-signal-${signal.name}'),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(heading, style: text.titleSmall),
        Text(signal.description, style: text.bodyMedium),
        if (signal.attributes.isNotEmpty)
          Text(
            [for (final attribute in signal.attributes) '${attribute.name}${attribute.optional ? '?' : ''}: ${attribute.shape}'].join('\n'),
            style: text.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
          ),
      ]),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.settings,
    builder: (context, _) {
      final s = widget.settings;
      final scheme = Theme.of(context).colorScheme;
      final theme = Theme.of(context).textTheme;
      final text = AppText.of(context);
      return Column(key: const Key('telemetry-section'), crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(text('telemetry.intro'), style: theme.bodyMedium?.copyWith(color: scheme.onSurfaceVariant)),
        const SizedBox(height: 12),
        if (s.problem case final problem?)
          Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(problem, key: const Key('telemetry-problem'), style: TextStyle(color: scheme.error))),
        if (s.loading && !s.loaded) const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()),
        if (s.loaded) ...[
          SettingsGroup(
            key: const Key('telemetry-recorded'),
            title: text('telemetry.recorded'),
            children: [
              for (final group in telemetryGroups) ...[
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
                  child: Text(text('telemetry.group.$group'), key: Key('telemetry-group-$group'), style: theme.labelLarge),
                ),
                for (final signal in s.signalsOf(group)) _signalRow(context, signal),
              ],
              Padding(padding: const EdgeInsets.all(16), child: Text(s.textRule, style: theme.bodySmall?.copyWith(color: scheme.onSurfaceVariant))),
            ],
          ),
          SettingsGroup(
            key: const Key('telemetry-never'),
            title: text('telemetry.never'),
            children: [
              for (final line in s.never)
                Padding(padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4), child: Text(line, key: Key('telemetry-never-${line.hashCode}'))),
            ],
          ),
          SettingsGroup(
            key: const Key('telemetry-switch'),
            title: text('telemetry.share'),
            children: [
              SettingsRow(
                title: text('telemetry.share'),
                description: _stateWords(text, s),
                control: Switch(
                  key: const Key('telemetry-enabled'),
                  value: s.enabled && s.accepted,
                  onChanged: s.exporters.isEmpty && !(s.enabled && s.accepted)
                      ? null
                      : (on) {
                          if (on) {
                            setState(() {
                              _reading = true;
                              _read = false;
                            });
                          } else {
                            unawaited(s.turnOff().then(_say));
                          }
                        },
                ),
              ),
              if (s.exporters.isEmpty) Padding(padding: const EdgeInsets.fromLTRB(16, 0, 16, 8), child: Text(text('telemetry.needs_exporter'), key: const Key('telemetry-needs-exporter'))),
              if (s.enabled && s.changed)
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
                  child: Row(children: [
                    Expanded(child: Text(text('telemetry.paused_changed'), key: const Key('telemetry-paused'), style: TextStyle(color: scheme.error))),
                    OutlinedButton(
                      key: const Key('telemetry-review'),
                      onPressed: () => setState(() {
                        _reading = true;
                        _read = false;
                      }),
                      child: Text(text('telemetry.review')),
                    ),
                  ]),
                ),
              if (_reading)
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
                  child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    CheckboxListTile(
                      key: const Key('telemetry-read'),
                      contentPadding: EdgeInsets.zero,
                      controlAffinity: ListTileControlAffinity.leading,
                      value: _read,
                      onChanged: (value) => setState(() => _read = value ?? false),
                      title: Text(text('telemetry.read')),
                      subtitle: Text(text('telemetry.digest', {'digest': s.digest}), key: const Key('telemetry-digest')),
                    ),
                    Row(children: [
                      FilledButton(key: const Key('telemetry-accept'), onPressed: _read ? () => unawaited(_accept()) : null, child: Text(text('telemetry.accept'))),
                      const SizedBox(width: 8),
                      TextButton(key: const Key('telemetry-accept-cancel'), onPressed: () => setState(() => _reading = false), child: Text(text('common.cancel'))),
                    ]),
                  ]),
                ),
            ],
          ),
          SettingsGroup(
            key: const Key('telemetry-groups'),
            title: text('telemetry.groups'),
            children: [
              for (final group in telemetryGroups)
                CheckboxListTile(
                  key: Key('telemetry-record-$group'),
                  dense: true,
                  value: s.record.contains(group),
                  title: Text(text('telemetry.group.$group')),
                  onChanged: (on) => unawaited(s.setRecorded(group, on: on ?? false).then(_say)),
                ),
            ],
          ),
          SettingsGroup(
            key: const Key('telemetry-exporters'),
            title: text('telemetry.exporters'),
            trailing: FilledButton.tonal(key: const Key('telemetry-add'), onPressed: s.providers.isEmpty ? null : () => unawaited(_addExporter()), child: Text(text('telemetry.add'))),
            children: [
              if (s.exporters.isEmpty) Padding(padding: const EdgeInsets.all(16), child: Text(text('telemetry.no_exporters'), key: const Key('telemetry-no-exporters'))),
              for (final exporter in s.exporters)
                SettingsRow(
                  key: Key('telemetry-exporter-${exporter.name}'),
                  title: [exporter.name, exporter.provider].join(' / '),
                  description: [
                    text('telemetry.state.${exporter.state}'),
                    text('telemetry.counts', {'queued': exporter.queued, 'dropped': exporter.dropped}),
                    if (exporter.lastError case final why?) text('telemetry.last_error', {'why': why}),
                  ].join('\n'),
                  control: Row(mainAxisSize: MainAxisSize.min, children: [
                    TextButton(key: Key('telemetry-test-${exporter.name}'), onPressed: () => unawaited(_test(exporter)), child: Text(text('telemetry.test'))),
                    IconButton(
                      key: Key('telemetry-remove-${exporter.name}'),
                      tooltip: text('telemetry.remove'),
                      icon: const Icon(Icons.delete_outline),
                      onPressed: () => unawaited(_removeExporter(exporter)),
                    ),
                  ]),
                ),
            ],
          ),
          if (_note case final note?) Padding(padding: const EdgeInsets.only(top: 8), child: Text(note, key: const Key('telemetry-note'), style: TextStyle(color: scheme.error))),
          if (s.restartPending)
            Card(
              key: const Key('telemetry-restart-note'),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Row(children: [
                  Expanded(child: Text(text('telemetry.restart_note'))),
                  if (s.restart != null) FilledButton(key: const Key('telemetry-restart'), onPressed: () => unawaited(s.restartHost()), child: Text(text('telemetry.restart'))),
                ]),
              ),
            ),
        ],
      ]);
    },
  );

  String _stateWords(AppText text, TelemetrySettings s) {
    if (s.state == 'running') return text('telemetry.state_running');
    if (s.state == 'paused' && s.paused != null) return text('telemetry.state_paused', {'why': s.paused!});
    return text('telemetry.state_off');
  }
}

/// The name, provider and settings of an exporter the person is adding.
final class _NewExporter {
  const _NewExporter(this.name, this.provider, this.settings);

  final String name, provider;
  final Map<String, Object?> settings;
}

class _AddExporterDialog extends StatefulWidget {
  const _AddExporterDialog({required this.providers});

  final List<TelemetryProvider> providers;

  @override
  State<_AddExporterDialog> createState() => _AddExporterDialogState();
}

class _AddExporterDialogState extends State<_AddExporterDialog> {
  final _name = TextEditingController();
  late TelemetryProvider _provider = widget.providers.first;
  Map<String, Object?> _values = const {};
  String? _problem;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _add() {
    final text = AppText.of(context);
    final name = _name.text.trim();
    if (!RegExp(r'^[a-z0-9_-]{1,32}$').hasMatch(name)) {
      setState(() => _problem = text('telemetry.name_rule'));
      return;
    }
    final problem = schemaProblem(_provider.schema, const {}, _values);
    if (problem != null) {
      setState(() => _problem = problem);
      return;
    }
    Navigator.pop(context, _NewExporter(name, _provider.id, collectSchemaValues(_provider.schema, const {}, _values)));
  }

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    return AlertDialog(
      title: Text(text('telemetry.add')),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            TextField(key: const Key('telemetry-new-name'), controller: _name, decoration: InputDecoration(labelText: text('telemetry.name'))),
            const SizedBox(height: 12),
            DropdownButtonFormField<TelemetryProvider>(
              key: const Key('telemetry-new-provider'),
              initialValue: _provider,
              decoration: InputDecoration(labelText: text('telemetry.provider')),
              items: [for (final provider in widget.providers) DropdownMenuItem(value: provider, child: Text(provider.title))],
              onChanged: (provider) => setState(() {
                _provider = provider ?? _provider;
                _values = const {};
              }),
            ),
            const SizedBox(height: 8),
            SchemaFields(key: ValueKey(_provider.id), schema: _provider.schema, form: const {}, values: _values, idPrefix: 'telemetry-new', onValues: (held) => _values = held),
            if (_problem case final problem?)
              Padding(padding: const EdgeInsets.only(top: 8), child: Text(problem, key: const Key('telemetry-new-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error))),
          ]),
        ),
      ),
      actions: [
        TextButton(key: const Key('telemetry-new-cancel'), onPressed: () => Navigator.pop(context), child: Text(text('common.cancel'))),
        FilledButton(key: const Key('telemetry-new-add'), onPressed: _add, child: Text(text('telemetry.add'))),
      ],
    );
  }
}
