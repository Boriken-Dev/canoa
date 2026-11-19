import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import 'programs.dart';
import 'programs_view.dart';
import 'runtime_mark.dart';
import 'runtime_page.dart';
import 'store_extension.dart';
import 'store_extension_catalog.dart';
import 'store_foundry.dart';
import 'store_runtimes.dart';
import 'store_selection.dart' show StoreMarkImage;

/// The icons an extension may name for its type; any other name draws the default. An extension may also ship a small picture of
/// its own as its icon ([extensionImage]).
const Map<String, IconData> extensionIcons = {
  'extension': Icons.extension_outlined,
  'palette': Icons.palette_outlined,
  'code': Icons.code,
  'build': Icons.build_outlined,
  'memory': Icons.memory,
  'folder': Icons.folder_outlined,
  'terminal': Icons.terminal,
  'apps': Icons.apps,
  'article': Icons.article_outlined,
  'book': Icons.menu_book_outlined,
  'bolt': Icons.bolt,
  'cloud': Icons.cloud_outlined,
  'database': Icons.storage_outlined,
  'download': Icons.download_outlined,
  'image': Icons.image_outlined,
  'language': Icons.language,
  'lock': Icons.lock_outline,
  'music': Icons.music_note_outlined,
  'science': Icons.science_outlined,
  'security': Icons.shield_outlined,
  'settings': Icons.settings_outlined,
  'star': Icons.star_outline,
  'storefront': Icons.storefront_outlined,
  'wrench': Icons.handyman_outlined,
  'tune': Icons.tune,
  'notifications': Icons.notifications_none,
  'mic': Icons.mic_none,
  'keyboard': Icons.keyboard_outlined,
  'hub': Icons.hub_outlined,
  'layers': Icons.layers_outlined,
  'accounts': Icons.manage_accounts_outlined,
  'mail': Icons.mail_outline,
  'speed': Icons.speed,
  'info': Icons.info_outline,
};

/// The picture an extension ships as its type's icon, from the `data:image/png;base64,` or `data:image/webp;base64,` address the
/// host gives; null for a name, or for an address that is not one of those. What it draws is checked again when it is shown.
StoreMarkImage? extensionImage(Object? icon) {
  if (icon is! String) return null;
  for (final (prefix, path) in const [('data:image/png;base64,', 'icon.png'), ('data:image/webp;base64,', 'icon.webp')]) {
    if (!icon.startsWith(prefix)) continue;
    try {
      return StoreMarkImage(path: path, bytes: base64Decode(icon.substring(prefix.length)));
    } on FormatException {
      return null;
    }
  }
  return null;
}

/// The store type made of the programs the host lists in [programs]: each store is an item, and + plans its install (or its
/// update, when the host's own copy is there), shows the plan for the person to confirm and runs it.
ExtensionStoreType programsStoreType({
  required String id,
  required String label,
  required IconData icon,
  StoreMarkImage? image,
  required ProgramStores programs,
  required String catalog,
  Future<void> Function(BuildContext context)? afterInstall,
}) => ExtensionStoreType(
  id: id,
  label: label,
  icon: icon,
  image: image,
  changes: programs,
  load: () => programs.refresh(latest: true),
  items: () => [
    for (final status in programs.statuses)
      ExtensionStoreItem(
        catalog: catalog,
        name: status.info.title,
        by: status.info.publisher,
        leading: RuntimeMark(icon: status.info.icon),
        details: (context, config) => RuntimePage(programs: programs, provider: status.info.provider, config: config),
        summary: status.installed
            ? 'Installed${status.version == null ? '' : ' ${status.version!}'}${status.hasManaged ? '' : ' (found on this computer)'}'
            : 'Not installed',
        installed: status.installed,
        install: (context) async {
          final done = await runProgramAction(context, programs, status, status.hasManaged ? 'update' : 'install');
          // What a program was installed for may need setting up next, as a secret provider's does.
          if (done && afterInstall != null && context.mounted) await afterInstall(context);
        },
        remove: status.hasManaged ? (context) => runProgramAction(context, programs, status, 'uninstall') : null,
      ),
  ],
);

/// The store types beyond the built-in four: Runtimes, the Extensions the catalogs list, each type an extension named in the host's
/// canoa.cfg adds and each a library the host carries adds. The host lists the types and does every install; this only holds what it
/// said. Foundry Local's models are listed while the host lists the type its library adds.
class StoreExtensions extends ChangeNotifier {
  StoreExtensions({
    required Future<ControlClient> Function() control,
    required Future<ControlClient> Function() open,
    bool Function()? mayInstall,
    Future<bool> Function()? restart,
    Future<void> Function()? saved,
    this.afterInstall,
  }) : _control = control,
       _open = open {
    _runtimes = ProgramStores(control: control, open: open, kind: 'runtimes');
    _foundry = FoundryModels(control: control, open: open);
    _catalogs = ExtensionCatalogs(control: control, mayInstall: mayInstall, restart: restart, saved: saved);
    _types = _built(const [], withModels: false);
  }

  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;

  /// What is done once a program of the type [id] was installed: the Secret providers type opens the provider's setup.
  final Future<void> Function(BuildContext context, String type)? afterInstall;
  late final ProgramStores _runtimes;
  late final FoundryModels _foundry;
  late final ExtensionCatalogs _catalogs;
  late List<ExtensionStoreType> _types;
  Future<void>? _asking;
  final _held = <ProgramStores>[];

  /// The runtimes the host's Store keeps, which the Runtimes type lists and an overlay's unmet need is installed from.
  ProgramStores get runtimes => _runtimes;

  /// Runtimes first, then Foundry Local's models (while the host lists its library's type) and the Extensions the catalogs list, then
  /// what extensions add, in the host's order.
  List<ExtensionStoreType> get types => _types;

  /// The titles of the types each extension adds, by the extension's name in canoa.cfg, as the host listed them.
  Map<String, List<String>> get addedBy => _addedBy;
  var _addedBy = <String, List<String>>{};

  List<ExtensionStoreType> _built(List<ExtensionStoreType> added, {required bool withModels}) => [
    _runtimesType(),
    if (withModels) foundryModelsType(_foundry),
    extensionsStoreType(_catalogs),
    ...added,
  ];

  ExtensionStoreType _runtimesType() => programsStoreType(
    id: runtimesTypeId,
    label: runtimesTypeLabel,
    icon: Icons.memory,
    programs: _runtimes,
    catalog: runtimesCatalog,
  );

  /// Asks the host which types extensions add, once. A host that refuses leaves Runtimes alone.
  Future<void> ensure() => _asking ??= _ask();

  /// Asks again, as after the host started with other extensions.
  Future<void> refresh() => _asking = _ask();

  Future<void> _ask() async {
    try {
      final client = await _control();
      final answer = await client.call('store/extensions', const {});
      final list = answer is Map && answer['types'] is List ? answer['types'] as List : const [];
      final added = <ExtensionStoreType>[];
      final by = <String, List<String>>{};
      final listing = <Future<void>>[];
      for (final each in list) {
        if (each is! Map || each['id'] is! String || each['title'] is! String) continue;
        final id = each['id'] as String;
        if (id == runtimesTypeId || id == foundryTypeId || id == extensionsTypeId) continue;
        final programs = ProgramStores(control: _control, open: _open, kind: 'extension', type: id);
        added.add(programsStoreType(
          id: id,
          label: each['title'] as String,
          icon: extensionIcons[each['icon']] ?? Icons.extension_outlined,
          image: extensionImage(each['icon']),
          programs: programs,
          catalog: each['extension_title'] is String ? each['extension_title'] as String : 'An extension',
          afterInstall: afterInstall == null ? null : (context) => afterInstall!(context, id),
        ));
        if (each['extension'] is String) (by[each['extension'] as String] ??= []).add(each['title'] as String);
        _held.add(programs);
        listing.add(programs.ensure());
      }
      _types = _built(added, withModels: added.any((type) => type.id == foundryLibraryTypeId));
      _addedBy = by;
      notifyListeners();
      await Future.wait([_runtimes.ensure(), ...listing]);
    } on Object {
      // The host offers no extensions: its Store has the types it has.
      _asking = null;
    }
  }

  @override
  void dispose() {
    _runtimes.dispose();
    _foundry.dispose();
    _catalogs.dispose();
    for (final programs in _held) {
      programs.dispose();
    }
    super.dispose();
  }
}
