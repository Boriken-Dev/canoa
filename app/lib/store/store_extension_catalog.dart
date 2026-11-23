import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart';
import '../selectable.dart';
import '../shell/confirm_remove.dart';
import 'programs.dart';
import 'programs_view.dart' show PlanDialog;
import 'store_extension.dart';

/// One extension an extensions catalog lists, as the host's `extension/catalogs` gives it: the catalog's claim, shown before
/// anything is fetched. The host computes the digest over what it fetches before it keeps any of it.
final class CatalogExtension {
  const CatalogExtension(
    this.name,
    this.origin, {
    required this.provider,
    this.pin,
    this.sha256,
    this.load,
    this.kind,
    this.version,
    this.platforms = const [],
    this.description,
    this.about = const {},
  });

  final String name;

  /// Where it comes from: `kind` is `git` (a tree: `location`, `ref`, `path`) or `url` (one file).
  final Map<String, Object?> origin;

  /// The identifier the extension offers itself under.
  final String provider;

  /// The tree digest of a Git source, or the SHA-256 of a URL's file: one of the two.
  final String? pin, sha256;

  /// For a tree, the library file in it the host loads.
  final String? load;

  /// What it needs of the host: the kind and its version, and the systems it runs on (every system when empty).
  final String? kind, version;
  final List<String> platforms;
  final String? description;

  /// The valid informational fields: `summary`, `homepage`, `repository`, `license`, `authors`, `tags`.
  final Map<String, Object?> about;
}

/// One extensions catalog of `[store.extensions.catalogs]`, as the host indexed it, or why it could not.
final class ExtensionCatalog {
  const ExtensionCatalog({
    required this.label,
    required this.extensions,
    this.problem,
  });

  final String label;
  final Map<String, CatalogExtension> extensions;
  final String? problem;
}

/// The catalogs in the answer of `extension/catalogs`, in the order the host gave them, each with the extensions it lists.
List<ExtensionCatalog> extensionCatalogsFromIndex(Object? answer) {
  if (answer is! Map<Object?, Object?>) return const [];
  final entries = [
    for (final each in answer['entries'] as List<Object?>? ?? const [])
      if (each is Map<Object?, Object?>) each,
  ];
  final found = <ExtensionCatalog>[];
  for (final each in answer['sources'] as List<Object?>? ?? const []) {
    if (each is! Map<Object?, Object?> ||
        each['kind'] != 'extension_catalog' ||
        each['label'] is! String) {
      continue;
    }
    final label = each['label']! as String;
    final listed = <String, CatalogExtension>{};
    for (final entry in entries) {
      final name = entry['name'];
      final origin = entry['origin'];
      final facts = entry['extension'];
      if (entry['catalog'] != label ||
          entry['kind'] != 'extension' ||
          name is! String) {
        continue;
      }
      if (origin is! Map<Object?, Object?> ||
          facts is! Map<Object?, Object?> ||
          facts['provider'] is! String) {
        continue;
      }
      final needs = facts['needs'];
      final about = entry['about'];
      listed[name] = CatalogExtension(
        name,
        {for (final MapEntry(:key, :value) in origin.entries) '$key': value},
        provider: facts['provider']! as String,
        pin: facts['pin'] as String?,
        sha256: facts['sha256'] as String?,
        load: facts['load'] as String?,
        kind: needs is Map<Object?, Object?> ? needs['kind'] as String? : null,
        version: needs is Map<Object?, Object?>
            ? needs['version'] as String?
            : null,
        platforms:
            needs is Map<Object?, Object?> &&
                needs['platforms'] is List<Object?>
            ? [
                for (final platform in needs['platforms']! as List<Object?>)
                  '$platform',
              ]
            : const [],
        description: entry['description'] as String?,
        about: {
          if (entry['summary'] case final String summary) 'summary': summary,
          if (entry['tags'] case final List<Object?> tags when tags.isNotEmpty)
            'tags': [for (final tag in tags) '$tag'],
          if (about is Map<Object?, Object?>)
            for (final MapEntry(:key, :value) in about.entries) '$key': value,
        },
      );
    }
    final problem = each['problem'];
    found.add(
      ExtensionCatalog(
        label: label,
        extensions: {
          for (final name in listed.keys.toList()..sort()) name: listed[name]!,
        },
        problem: problem is String && listed.isEmpty ? problem : null,
      ),
    );
  }
  return found;
}

/// The Store's own type for the extensions the catalogs list: an extension is code the host loads, so it is installed through the
/// host's plan and confirm, which says where it comes from and the digest the host checks what it fetches against.
const extensionsTypeId = 'extensions';
const extensionsTypeLabel = 'Extensions';

/// The extensions the catalogs list and the ones the host's canoa.cfg names, for the Extensions type. The host fetches, checks and
/// keeps an install and writes `[host.extensions.NAME]` (which takes the grant that guards `[host]`); this only asks and shows.
class ExtensionCatalogs extends ChangeNotifier {
  ExtensionCatalogs({
    required this._control,
    this.mayInstall,
    this.restart,
    this.saved,
  });

  final Future<ControlClient> Function() _control;

  /// Whether this key may install and remove: it holds the grant that guards `[host]`. Null is yes, and the host refuses when not.
  final bool Function()? mayInstall;

  /// Has the host start again so it loads what changed; true when it did. Null where this page cannot.
  final Future<bool> Function()? restart;

  /// Runs after canoa.cfg was changed through the host, so what the app holds of it is read again.
  final Future<void> Function()? saved;

  var _catalogs = const <ExtensionCatalog>[];
  var _installed = const <String>{};
  String? _problem;
  bool _disposed = false;

  /// Why the catalogs could not be read, in the host's words; null when they were.
  String? get problem => _problem;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads the catalogs the host indexed, and which extensions its canoa.cfg names. [refresh] has the host ask its sources again.
  Future<void> load({bool refresh = false}) async {
    try {
      final client = await _control();
      _catalogs = extensionCatalogsFromIndex(
        await client.call(
          'extension/catalogs',
          {if (refresh) 'refresh': true},
          null,
          const Duration(minutes: 6),
        ),
      );
      _problem = null;
    } on Object catch (error) {
      _problem = hostWords(error);
    }
    await _readInstalled();
    notifyListeners();
  }

  Future<void> _readInstalled() async {
    try {
      final answer = await (await _control()).call('config/get', {
        'section': 'host',
      });
      final host = answer is Map ? answer['value'] : null;
      final table = host is Map ? host['extensions'] : null;
      _installed = table is Map
          ? {for (final key in table.keys) '$key'}
          : const {};
    } on Object {
      // A key the host does not let read [host] shows none as installed; installing is refused in the host's words.
      _installed = const {};
    }
  }

  /// A tile for each extension each catalog lists.
  List<ExtensionStoreItem> items() {
    final may = mayInstall?.call() ?? true;
    return [
      for (final loaded in _catalogs)
        for (final extension in loaded.extensions.values)
          ExtensionStoreItem(
            catalog: loaded.label,
            name: extension.name,
            by: loaded.label,
            summary:
                (extension.about['summary'] as String?) ??
                extension.description,
            tags: [
              if (extension.about['tags'] case final List<Object?> tags)
                for (final tag in tags) '$tag',
            ],
            installed: _installed.contains(extension.name),
            install: may
                ? (context) => _install(context, loaded.label, extension)
                : null,
            remove: may && _installed.contains(extension.name)
                ? (context) => _remove(context, extension.name)
                : null,
          ),
    ];
  }

  Future<void> _install(
    BuildContext context,
    String catalog,
    CatalogExtension extension,
  ) async {
    final client = await _control();
    final ProgramPlan plan;
    try {
      plan = ProgramPlan.parse(
        extension.name,
        await client.call('extension/plan', {
          'catalog': catalog,
          'name': extension.name,
        }),
      );
    } on Object catch (error) {
      if (context.mounted) {
        await _showProblem(
          context,
          'Install ${extension.name}',
          hostWords(error),
        );
      }
      return;
    }
    if (!context.mounted) return;
    final confirmed = await showSelectableDialog<bool>(
      context: context,
      builder: (_) => PlanDialog(plan: plan, title: extension.name),
    );
    if (confirmed != true || !context.mounted) return;
    final navigator = Navigator.of(context, rootNavigator: true);
    unawaited(
      showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => _Working(title: 'Installing ${extension.name}'),
      ),
    );
    try {
      await client.call('extension/apply', {'plan': plan.raw}, null, applyWait);
    } on Object catch (error) {
      navigator.pop();
      if (context.mounted) {
        await _showProblem(
          context,
          'Install ${extension.name}',
          hostWords(error),
        );
      }
      await load();
      return;
    }
    navigator.pop();
    await load();
    await saved?.call();
    if (context.mounted) {
      await _offerRestart(context, 'Installed ${extension.name}');
    }
  }

  Future<void> _remove(BuildContext context, String name) async {
    if (!await confirmRemoval(
      context,
      title: 'Remove $name',
      body: 'The extension goes out of the host\'s canoa.cfg, and its files out of the folder the host kept them in. Its types leave the '
          'Store when the host restarts.',
      cancelLabel: 'Cancel',
      dialogKey: const Key('extension-store-remove-dialog'),
      cancelKey: null,
      confirmKey: const Key('extension-store-remove-confirm'),
      selectable: true,
    ) || !context.mounted) {
      return;
    }
    try {
      await (await _control()).call('extension/remove', {'name': name});
    } on Object catch (error) {
      if (context.mounted) {
        await _showProblem(context, 'Remove $name', hostWords(error));
      }
      return;
    }
    await load();
    await saved?.call();
    if (context.mounted) await _offerRestart(context, 'Removed $name');
  }

  /// Says the host reads extensions only when it starts, and offers to start it again.
  Future<void> _offerRestart(BuildContext context, String title) async {
    final restart = this.restart;
    final now = await showSelectableDialog<bool>(
      context: context,
      builder: (dialog) => AlertDialog(
        key: const Key('extension-store-restart-dialog'),
        title: Text(title),
        content: const Text(
          'The host reads its extensions when it starts: it restarts to load the change, and the sessions open on it end.',
        ),
        actions: [
          TextButton(
            key: const Key('extension-store-restart-later'),
            onPressed: () => Navigator.pop(dialog, false),
            child: const Text('Later'),
          ),
          if (restart != null)
            FilledButton(
              key: const Key('extension-store-restart-now'),
              onPressed: () => Navigator.pop(dialog, true),
              child: const Text('Restart the host'),
            ),
        ],
      ),
    );
    if (now == true) await restart?.call();
  }

  Future<void> _showProblem(
    BuildContext context,
    String title,
    String message,
  ) => showSelectableDialog<void>(
    context: context,
    builder: (dialog) => AlertDialog(
      key: const Key('extension-store-error-dialog'),
      title: Text(title),
      content: Text(message, key: const Key('extension-store-error')),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(dialog),
          child: const Text('Close'),
        ),
      ],
    ),
  );
}

class _Working extends StatelessWidget {
  const _Working({required this.title});

  final String title;

  @override
  Widget build(BuildContext context) => AlertDialog(
    key: const Key('extension-store-working'),
    title: Text(title),
    content: const Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        LinearProgressIndicator(),
        SizedBox(height: 8),
        Text(
          'The host fetches it and checks it against the digest before it keeps any of it.',
        ),
      ],
    ),
  );
}

/// The store type the extensions of [catalogs] make.
ExtensionStoreType extensionsStoreType(ExtensionCatalogs catalogs) =>
    ExtensionStoreType(
      id: extensionsTypeId,
      label: extensionsTypeLabel,
      icon: Icons.extension_outlined,
      items: catalogs.items,
      changes: catalogs,
      load: catalogs.load,
    );
