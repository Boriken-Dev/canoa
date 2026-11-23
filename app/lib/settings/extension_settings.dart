import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../served/host_files.dart';
import '../shell/confirm_remove.dart';
import 'settings_row.dart';

/// One `[host.extensions.NAME]` of the host's canoa.cfg: a store extension the host loads, which adds types to the Store.
final class ExtensionEntry {
  const ExtensionEntry({required this.name, required this.table});

  final String name;

  /// The table as the host holds it, every key of it: an edit here changes the keys this page knows and keeps the others.
  final Map<String, Object?> table;

  String get provider => table['provider'] is String ? table['provider']! as String : '';
  String? get library => table['library'] is String ? table['library'] as String : null;
  String? get source => table['source'] is String ? table['source'] as String : null;

  /// Where the extension is loaded from.
  String get where => library ?? source ?? '';

  /// Whether the Store installed it, so the host keeps the digest it checked and checks it again each time the extension is loaded.
  bool get fromStore => table['pin'] is String || table['sha256'] is String;
}

/// The extensions of the host canoa.cfg names, listed, added, changed and removed through the host's own `[host]` table: the host
/// checks what canoa.cfg would then hold before it writes it, and reads the table only when it starts, so [restartPending] is
/// true once something changed. Needs the grant that guards `[host]`, which is as much as the programs the host runs.
class ExtensionSettings extends ChangeNotifier {
  ExtensionSettings({required this._files, this.types, this.restart, this.saved});

  final HostFiles _files;

  /// The titles of the store types each extension adds, by its name; asked each time the list is read.
  final Future<Map<String, List<String>>> Function()? types;

  /// Has the host start again, which loads what was changed; null where this page cannot.
  final Future<bool> Function()? restart;

  /// Runs after canoa.cfg was written, so what the app holds of it is read again.
  final Future<void> Function()? saved;

  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  List<ExtensionEntry> _entries = const [];
  Map<String, List<String>> _adds = const {};
  String? _problem;
  bool _loading = false, _loaded = false, _restartPending = false;

  List<ExtensionEntry> get entries => _entries;

  /// The types of the Store the extension [name] added when the host last started; empty when it added none.
  List<String> addedBy(String name) => _adds[name] ?? const [];

  /// Why the list could not be read or the last change was refused, in the host's words; null when there is nothing to say.
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;

  /// Whether canoa.cfg now names extensions the running host has not loaded yet.
  bool get restartPending => _restartPending;

  /// Reads the extensions from the host.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      final host = await _files.configGet('host');
      _entries = _entriesOf(host);
      _problem = null;
      _loaded = true;
      if (types != null) {
        try {
          _adds = await types!();
        } on Object {
          // What an extension adds is a note on its row, not what the list is made of.
        }
      }
    } on ControlFailed catch (error) {
      _problem = error.code == -32001 ? "This key may not change the host's own settings." : error.message;
    } on Object catch (error) {
      _problem = 'The host could not be reached: $error';
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  static List<ExtensionEntry> _entriesOf(Map<String, Object?>? host) {
    final table = host?['extensions'];
    if (table is! Map) return const [];
    return [
      for (final MapEntry(:key, :value) in table.entries)
        if (value is Map) ExtensionEntry(name: '$key', table: {for (final MapEntry(:key, :value) in value.entries) '$key': value}),
    ];
  }

  /// Adds the extension [name], or changes the one called [replacing]. [library] or [source] says where it is loaded from and
  /// [provider] which of its providers adds the types. Returns what is wrong in the host's words, or null once canoa.cfg holds it.
  Future<String?> save({required String name, String? library, String? source, required String provider, String? replacing}) async {
    name = name.trim();
    provider = provider.trim();
    final from = [library?.trim() ?? '', source?.trim() ?? ''];
    if (name.isEmpty) return 'An extension needs a name.';
    if (provider.isEmpty) return 'An extension needs the id of the provider that adds its types.';
    if (from.where((each) => each.isNotEmpty).length != 1) return 'Name the library file or the source the extension is loaded from, one of them.';
    return _change((extensions) {
      if (name != replacing && extensions.containsKey(name)) return 'There is an extension called $name already.';
      final old = replacing == null ? null : extensions[replacing];
      final kept = <String, Object?>{
        if (old is Map)
          for (final MapEntry(:key, :value) in old.entries) '$key': value,
      };
      // A library or source that is not the one the Store checked is not covered by the digest it kept.
      final moved = (old is Map ? old['library'] ?? old['source'] : null) != (from[0].isNotEmpty ? from[0] : from[1]);
      final table = <String, Object?>{
        for (final MapEntry(:key, :value) in kept.entries)
          if (key != 'library' && key != 'source' && !(moved && (key == 'pin' || key == 'sha256' || key == 'root'))) key: value,
        if (from[0].isNotEmpty) 'library': from[0] else 'source': from[1],
        'provider': provider,
      };
      final next = <String, Object?>{};
      if (replacing != null && extensions.containsKey(replacing)) {
        for (final MapEntry(:key, :value) in extensions.entries) {
          if (key == replacing) {
            next[name] = table;
          } else {
            next[key] = value;
          }
        }
      } else {
        next
          ..addAll(extensions)
          ..[name] = table;
      }
      return next;
    });
  }

  /// Takes the extension [name] out of canoa.cfg, and its files with it when the Store installed it. Returns what is wrong in the
  /// host's words, or null.
  Future<String?> remove(String name) async {
    if (_entries.where((each) => each.name == name).firstOrNull?.fromStore ?? false) {
      try {
        await _files.extensionRemove(name);
        _restartPending = true;
        _problem = null;
        if (saved != null) await saved!();
        await load();
        return null;
      } on ControlFailed catch (error) {
        return error.code == -32001 ? "This key may not change the host's own settings." : error.message;
      } on Object catch (error) {
        return 'The host could not be reached to remove that: $error';
      }
    }
    return _change((extensions) {
      if (!extensions.containsKey(name)) return 'There is no extension called $name.';
      return {
        for (final MapEntry(:key, :value) in extensions.entries)
          if (key != name) key: value,
      };
    });
  }

  /// Reads `[host]` again from the host, lets [edit] make the extensions it should name (or say, as text, why not), has the host
  /// check the result and writes it. Reading it again keeps another page's change to the table.
  Future<String?> _change(Object Function(Map<String, Object?> extensions) edit) async {
    try {
      final host = {...?await _files.configGet('host')};
      final current = host['extensions'];
      final extensions = <String, Object?>{
        if (current is Map)
          for (final MapEntry(:key, :value) in current.entries) '$key': value,
      };
      final made = edit(extensions);
      if (made is String) return made;
      final next = made as Map<String, Object?>;
      if (next.isEmpty) {
        host.remove('extensions');
      } else {
        host['extensions'] = next;
      }
      final value = host.isEmpty ? null : host;
      final wrong = await _files.configCheck('host', value);
      if (wrong != null) return wrong;
      await _files.configSet('host', value);
      _restartPending = true;
      _problem = null;
      if (saved != null) await saved!();
      await load();
      return null;
    } on ControlFailed catch (error) {
      return error.code == -32001 ? "This key may not change the host's own settings." : error.message;
    } on Object catch (error) {
      return 'The host could not be reached to save that: $error';
    }
  }

  /// Has the host start again so that it loads the extensions now named.
  Future<void> restartHost() async {
    if (restart == null) return;
    if (!await restart!()) return;
    _restartPending = false;
    notifyListeners();
    await load();
  }
}

/// Settings' Extensions page: the store extensions the host loads, with Add, Edit and Remove.
class ExtensionsSection extends StatefulWidget {
  const ExtensionsSection({super.key, required this.settings});

  final ExtensionSettings settings;

  @override
  State<ExtensionsSection> createState() => _ExtensionsSectionState();
}

class _ExtensionsSectionState extends State<ExtensionsSection> {
  ExtensionSettings get settings => widget.settings;

  @override
  void initState() {
    super.initState();
    unawaited(settings.load());
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: settings,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      final text = Theme.of(context).textTheme;
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(
          'An extension adds types to the Store, each made of programs the host installs with the plan you confirm. The host loads '
          'extensions when it starts. The Store\'s Extensions type installs the ones the catalogs list, checking what it fetches against '
          'their digest; the ones named here by hand are loaded as they are.',
          style: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant),
        ),
        const SizedBox(height: 12),
        if (settings.problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text(problem, key: const Key('extensions-problem'), style: TextStyle(color: scheme.error)),
          ),
        if (settings.restartPending)
          Card(
            key: const Key('extensions-restart-note'),
            child: Padding(
              padding: const EdgeInsets.all(12),
              child: Row(children: [
                const Expanded(child: Text('Saved. The host restarts to load the change; the sessions open on it end.')),
                if (settings.restart != null)
                  FilledButton(
                    key: const Key('extensions-restart'),
                    onPressed: () => unawaited(settings.restartHost()),
                    child: const Text('Restart the host'),
                  ),
              ]),
            ),
          ),
        if (settings.loading && !settings.loaded) const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()),
        if (settings.loaded && settings.entries.isEmpty)
          const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('No extension is named in the host\'s canoa.cfg.', key: Key('extensions-none'))),
        for (final entry in settings.entries)
          SettingsRow(
            key: Key('extension-row-${entry.name}'),
            title: entry.name,
            description: _describe(entry),
            control: Row(mainAxisSize: MainAxisSize.min, children: [
              IconButton(
                key: Key('extension-edit-${entry.name}'),
                tooltip: 'Edit',
                icon: const Icon(Icons.edit_outlined),
                onPressed: () => unawaited(_edit(context, entry)),
              ),
              IconButton(
                key: Key('extension-remove-${entry.name}'),
                tooltip: 'Remove',
                icon: const Icon(Icons.delete_outline),
                onPressed: () => unawaited(_remove(context, entry)),
              ),
            ]),
          ),
        const SizedBox(height: 12),
        Align(
          alignment: Alignment.centerLeft,
          child: OutlinedButton.icon(
            key: const Key('extension-add'),
            onPressed: settings.loaded ? () => unawaited(_edit(context, null)) : null,
            icon: const Icon(Icons.add),
            label: const Text('Add an extension'),
          ),
        ),
      ]);
    },
  );

  String _describe(ExtensionEntry entry) {
    final adds = settings.addedBy(entry.name);
    return [
      '${entry.provider} from ${entry.where}',
      if (entry.fromStore) 'Installed from a catalog: its digest is checked each time the host loads it',
      if (adds.isNotEmpty) 'Adds ${adds.join(', ')}' else if (settings.restartPending) 'Loads when the host restarts' else 'Adds nothing: it did not load, or names none',
    ].join('\n');
  }

  Future<void> _remove(BuildContext context, ExtensionEntry entry) async {
    if (!await confirmRemoval(
      context,
      title: 'Remove ${entry.name}',
      body: 'The extension goes out of the host\'s canoa.cfg, and its types leave the Store when the host restarts. Programs it installed '
          'stay where they are.',
      cancelLabel: 'Cancel',
      dialogKey: const Key('extension-remove-dialog'),
      cancelKey: null,
      confirmKey: const Key('extension-remove-confirm'),
    ) || !context.mounted) {
      return;
    }
    final problem = await settings.remove(entry.name);
    if (problem != null && context.mounted) {
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(problem)));
    }
  }

  Future<void> _edit(BuildContext context, ExtensionEntry? entry) => showDialog<void>(
    context: context,
    builder: (dialog) => _ExtensionDialog(settings: settings, entry: entry),
  );
}

class _ExtensionDialog extends StatefulWidget {
  const _ExtensionDialog({required this.settings, required this.entry});

  final ExtensionSettings settings;
  final ExtensionEntry? entry;

  @override
  State<_ExtensionDialog> createState() => _ExtensionDialogState();
}

class _ExtensionDialogState extends State<_ExtensionDialog> {
  late final _name = TextEditingController(text: widget.entry?.name ?? '');
  late final _library = TextEditingController(text: widget.entry?.library ?? '');
  late final _source = TextEditingController(text: widget.entry?.source ?? '');
  late final _provider = TextEditingController(text: widget.entry?.provider ?? '');
  String? _problem;
  var _saving = false;

  @override
  void dispose() {
    _name.dispose();
    _library.dispose();
    _source.dispose();
    _provider.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _problem = null;
    });
    final problem = await widget.settings.save(
      name: _name.text,
      library: _library.text,
      source: _source.text,
      provider: _provider.text,
      replacing: widget.entry?.name,
    );
    if (!mounted) return;
    if (problem == null) {
      Navigator.pop(context);
    } else {
      setState(() {
        _saving = false;
        _problem = problem;
      });
    }
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    key: const Key('extension-dialog'),
    title: Text(widget.entry == null ? 'Add an extension' : 'Edit ${widget.entry!.name}'),
    content: SizedBox(
      width: 420,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          TextField(key: const Key('extension-name'), controller: _name, decoration: const InputDecoration(labelText: 'Name')),
          TextField(
            key: const Key('extension-library'),
            controller: _library,
            decoration: const InputDecoration(labelText: 'Library file', helperText: 'A path on the host. Name this or a source.'),
          ),
          TextField(
            key: const Key('extension-source'),
            controller: _source,
            decoration: const InputDecoration(labelText: 'Source', helperText: 'A Guatiao source the host runs. Name this or a library.'),
          ),
          TextField(
            key: const Key('extension-provider'),
            controller: _provider,
            decoration: const InputDecoration(labelText: 'Provider', helperText: 'The id of the store_extension it offers.'),
          ),
          if (_problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(problem, key: const Key('extension-dialog-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
        ]),
      ),
    ),
    actions: [
      TextButton(onPressed: _saving ? null : () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(key: const Key('extension-save'), onPressed: _saving ? null : () => unawaited(_save()), child: const Text('Save')),
    ],
  );
}
