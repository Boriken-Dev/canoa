import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart';
import 'programs.dart';
import 'programs_view.dart';
import 'runtime_mark.dart';
import 'runtime_page.dart';
import 'store_extension.dart';
import 'store_extension_catalog.dart';
import 'store_models.dart';
import 'store_runtimes.dart';
import 'store_selection.dart' show StoreMarkImage;

/// The icons an extension may name for a type or a page it adds; any other name draws the default.
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
  'schedule': Icons.schedule,
  'info': Icons.info_outline,
};

/// The store type made of the programs the host lists in [programs]: each store is an item, grouped under the library that offers
/// it, and + plans its install (or its update, when the host's own copy is there), shows the plan for the person to confirm and runs it.
ExtensionStoreType programsStoreType({
  required String id,
  required String label,
  required IconData icon,
  StoreMarkImage? image,
  required ProgramStores programs,
  required String catalog,
  Future<void> Function(BuildContext context, String type)? afterInstall,
}) => ExtensionStoreType(
  id: id,
  label: label,
  icon: icon,
  image: image,
  changes: programs,
  load: () => programs.refresh(latest: true),
  updates: (text) => programUpdates(programs, text),
  items: () => [
    for (final status in programs.statuses)
      ExtensionStoreItem(
        catalog: status.info.libraryTitle ?? status.info.library ?? catalog,
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
          final type = status.info.type;
          if (done && afterInstall != null && type != null && context.mounted) await afterInstall(context, type);
        },
        remove: status.hasManaged ? (context) => runProgramAction(context, programs, status, 'uninstall') : null,
      ),
  ],
);

/// The store types beyond the built-in four: Programs (every program a library offers, runtimes and a type's alike), Models (what
/// the libraries that list models list), and the Extensions the catalogs list. The host lists the programs and the models and does
/// every install; this only holds what it said.
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
    _programs = ProgramStores(control: control, open: open, kind: 'programs');
    _catalogs = ExtensionCatalogs(control: control, mayInstall: mayInstall, restart: restart, saved: saved);
    _types = _built(const []);
  }

  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;

  /// What is done once a program a type claims was installed: the Secret providers type opens the provider's setup.
  final Future<void> Function(BuildContext context, String type)? afterInstall;
  late final ProgramStores _programs;
  late final ExtensionCatalogs _catalogs;
  late List<ExtensionStoreType> _types;
  Future<void>? _asking;
  final _heldModels = <TypeModels>[];

  /// The programs the host's Store keeps, which the Programs type lists and an overlay's unmet need is installed from.
  ProgramStores get programs => _programs;

  /// Programs first, then the models the libraries list (when any does), then the Extensions the catalogs list.
  List<ExtensionStoreType> get types => _types;

  /// The titles of the types each extension adds, by the extension's name in canoa.cfg, as the host listed them.
  Map<String, List<String>> get addedBy => _addedBy;
  var _addedBy = <String, List<String>>{};

  List<ExtensionStoreType> _built(List<TypeModels> models) => [
    programsStoreType(
      id: programsTypeId,
      label: programsTypeLabel,
      icon: Icons.memory,
      programs: _programs,
      catalog: programsCatalog,
      afterInstall: afterInstall,
    ),
    if (models.isNotEmpty) modelsStoreType(models: models),
    extensionsStoreType(_catalogs),
  ];

  /// Asks the host which types extensions add, once. A host that refuses leaves Programs alone.
  Future<void> ensure() => _asking ??= _ask();

  /// Asks again, as after the host started with other extensions.
  Future<void> refresh() => _asking = _ask();

  Future<void> _ask() async {
    try {
      final client = await _control();
      final answer = await client.call('store/extensions', const {});
      final list = answer is Map && answer['types'] is List ? answer['types'] as List : const [];
      final models = <TypeModels>[];
      final by = <String, List<String>>{};
      for (final each in list) {
        if (each is! Map || each['id'] is! String || each['title'] is! String) continue;
        final id = each['id'] as String;
        if (id == programsTypeId || id == modelsTypeId || id == extensionsTypeId) continue;
        if (each['models'] case {'title': final String title}) {
          models.add(TypeModels(type: id, title: title, control: _control, open: _open, programs: _programs));
        }
        if (each['extension'] is String) (by[each['extension'] as String] ??= []).add(each['title'] as String);
      }
      _heldModels.addAll(models);
      _types = _built(models);
      _addedBy = by;
      notifyListeners();
      await _programs.ensure();
    } on Object {
      // The host offers no extensions: its Store has the types it has.
      _asking = null;
    }
  }

  @override
  void dispose() {
    _programs.dispose();
    for (final models in _heldModels) {
      models.dispose();
    }
    _catalogs.dispose();
    super.dispose();
  }
}
