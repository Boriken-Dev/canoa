/// The sections of Settings the host's providers offer (the `settings_section` kind): each says its name, icon, where it sits (a section
/// of its own under a heading, or a group inside an existing section) and the form it asks for, a guatiao schema with the guatiao-intake
/// form that shows it. The app draws it with the one generic renderer and knows the section only by what its provider says of it.
library;

import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../store/store_extensions.dart' show extensionIcons;
import 'built_in_sections.dart';
import 'settings_row.dart';

/// What the app asks the host with: `settings/sections`, `settings/section_get` and `settings/section_set`.
typedef SectionCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// The heading a section sits under when its provider names none.
const defaultSectionGroup = 'Extensions';

/// One section a provider offers.
final class HostSection {
  const HostSection({
    required this.id,
    required this.title,
    required this.origin,
    this.help,
    this.icon,
    this.group,
    this.under,
    this.keywords = '',
    this.stored = 'config',
    this.schema = const {},
    this.form = const {},
  });

  final String id;
  final String title;

  /// Where its provider comes from: `bundled`, or the name of the entry or extension that loads it.
  final String origin;
  final String? help;
  final String? icon;

  /// The heading it sits under when it is a section of its own.
  final String? group;

  /// The id of the section it is a group in, when it is not a section of its own.
  final String? under;
  final String keywords;
  final String stored;
  final Map<String, Object?> schema;
  final Map<String, Object?> form;

  IconData get iconData => extensionIcons[icon] ?? Icons.extension_outlined;

  static HostSection? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    Map<String, Object?> map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};
    String? text(Object? value) => value is String && value.isNotEmpty ? value : null;
    return HostSection(
      id: raw['id'] as String,
      title: raw['title'] as String,
      origin: text(raw['origin']) ?? '',
      help: text(raw['help']),
      icon: text(raw['icon']),
      group: text(raw['group']),
      under: text(raw['under']),
      keywords: text(raw['keywords']) ?? '',
      stored: text(raw['stored']) ?? 'config',
      schema: map(raw['schema']),
      form: map(raw['form']),
    );
  }
}

/// The values a section holds, a secret left out and named in [secretsSet] instead. The host keeps a secret as a named secret of the
/// secret providers; [secretsProblem] says why one could not be asked, and then it is not named.
typedef SectionValues = ({Map<String, Object?> values, Set<String> secretsSet, String? secretsProblem});

/// The sections the host lists, and the values of each.
final class HostSections extends ChangeNotifier {
  /// Until the host answers, and for good when it cannot or will not, the sections are the application's own pages: what the host lists
  /// of them, kept in [builtInSectionsListing], so Settings is never empty and the person's own preferences show whatever the host
  /// grants.
  HostSections(this._call) : _sections = builtIn();

  /// The application's own pages, as the host lists them.
  static List<HostSection> builtIn() {
    final raw = (jsonDecode(builtInSectionsListing) as Map)['sections'] as List;
    return [for (final each in raw) ?HostSection.read(each)];
  }

  /// The sections of an app that has no host to ask.
  HostSections.withoutHost() : this((method, params) async => throw const ControlFailed(-32601, 'this app has no host'));

  final SectionCall _call;
  List<HostSection> _sections;
  bool _disposed = false;

  /// The sections in the order the host listed them.
  List<HostSection> get sections => _sections;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// The sections of their own, those that name no section to sit in, and those whose `under` is none of [known].
  List<HostSection> ownOf(Set<String> known) => [
    for (final section in _sections)
      if (section.under == null || !known.contains(section.under)) section,
  ];

  /// The groups that sit inside the section [id].
  List<HostSection> groupsIn(String id) => [
    for (final section in _sections)
      if (section.under == id) section,
  ];

  /// Takes the host's answer to `settings/sections`.
  void adopt(Object? answer) {
    final raw = answer is Map ? answer['sections'] : null;
    _sections = [if (raw is List) for (final each in raw) ?HostSection.read(each)];
    notifyListeners();
  }

  /// Asks the host for the sections again; a host that cannot answer or refuses leaves the list as it was.
  Future<void> load() async {
    try {
      adopt(await _call('settings/sections', const {}));
    } on Object {
      return;
    }
  }

  /// What the section [id] holds.
  Future<SectionValues> values(String id) async {
    final answer = await _call('settings/section_get', {'id': id});
    final map = answer is Map ? answer : const {};
    return (
      values: map['values'] is Map ? (map['values'] as Map).cast<String, Object?>() : const <String, Object?>{},
      secretsSet: {if (map['secrets_set'] is List) for (final each in map['secrets_set'] as List) if (each is String) each},
      secretsProblem: map['secrets_problem'] is String ? map['secrets_problem'] as String : null,
    );
  }

  /// Keeps [values] for the section [id]; [clear] names the secrets to take out. Returns why the host refused, or null once it is kept.
  Future<String?> save(String id, Map<String, Object?> values, {List<String> clear = const []}) async {
    try {
      await _call('settings/section_set', {'id': id, 'values': values, if (clear.isNotEmpty) 'clear': clear});
      return null;
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return 'The host could not be reached: $error';
    }
  }
}

/// The form of one section, drawn by [SchemaFields], with the section's help above it and Save under it.
///
/// With [asGroup] it is a group inside another section's page: a titled block there instead of a page.
class HostSectionForm extends StatefulWidget {
  const HostSectionForm({super.key, required this.section, required this.sections, this.asGroup = false});

  final HostSection section;
  final HostSections sections;
  final bool asGroup;

  @override
  State<HostSectionForm> createState() => _HostSectionFormState();
}

class _HostSectionFormState extends State<HostSectionForm> {
  Map<String, Object?>? _values;
  Set<String> _secretsSet = const {};
  String? _secretsProblem;
  final _typed = <String, Object?>{};
  final _cleared = <String>{};
  String? _problem;
  bool _saving = false, _saved = false;
  int _revision = 0;

  String get _id => widget.section.id;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final held = await widget.sections.values(_id);
      if (!mounted) return;
      setState(() {
        _values = held.values;
        _secretsSet = held.secretsSet;
        _secretsProblem = held.secretsProblem;
        _typed
          ..clear()
          ..addAll(held.values);
        _cleared.clear();
        _revision++;
      });
    } on ControlFailed catch (error) {
      if (mounted) setState(() => _problem = error.message);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = 'The host could not be reached: $error');
    }
  }

  Future<void> _save() async {
    final section = widget.section;
    final why = schemaProblem(section.schema, section.form, _typed);
    if (why != null) {
      setState(() {
        _problem = why;
        _saved = false;
      });
      return;
    }
    setState(() {
      _saving = true;
      _problem = null;
      _saved = false;
    });
    final problem = await widget.sections.save(_id, collectSchemaValues(section.schema, section.form, _typed), clear: _cleared.toList());
    if (!mounted) return;
    setState(() {
      _saving = false;
      _problem = problem;
      _saved = problem == null;
    });
    if (problem == null) await _load();
  }

  @override
  Widget build(BuildContext context) {
    final section = widget.section;
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final fields = _values == null
        ? (_problem == null ? const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()) : const SizedBox())
        : Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            SchemaFields(
              key: ValueKey('$_id-$_revision'),
              schema: section.schema,
              form: section.form,
              values: _values!,
              idPrefix: 'section-$_id',
              secretsSet: _secretsSet,
              cleared: _cleared,
              onValues: (values) => _typed
                ..clear()
                ..addAll(values),
              onClear: (key, {required clear}) => setState(() => clear ? _cleared.add(key) : _cleared.remove(key)),
            ),
            if (schemaProperties(section.schema).isNotEmpty)
              Align(
                alignment: Alignment.centerRight,
                child: Padding(
                  padding: const EdgeInsets.all(8),
                  child: Row(mainAxisSize: MainAxisSize.min, children: [
                    if (_saved) Padding(padding: const EdgeInsets.only(right: 12), child: Text('Saved', key: Key('section-$_id-saved'), style: text.bodySmall)),
                    FilledButton(key: Key('section-$_id-save'), onPressed: _saving ? null : () => unawaited(_save()), child: const Text('Save')),
                  ]),
                ),
              ),
          ]);
    final body = Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      if (section.help != null)
        Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(section.help!, style: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant))),
      if (_problem case final problem?)
        Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(problem, key: Key('section-$_id-problem'), style: TextStyle(color: scheme.error))),
      if (_secretsProblem case final problem?)
        Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Text('A secret of this section could not be asked of its secret provider: $problem', key: Key('section-$_id-secrets-problem'), style: TextStyle(color: scheme.error)),
        ),
      fields,
    ]);
    if (!widget.asGroup) return body;
    return SettingsGroup(key: Key('settings-group-$_id'), title: section.title, leading: Icon(section.iconData, size: 18), children: [Padding(padding: const EdgeInsets.all(12), child: body)]);
  }
}
