import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import 'programs.dart';
import 'programs_view.dart';
import 'store_extension.dart';
import 'store_runtimes.dart';

/// The icons an extension may name for its type; any other name draws the default.
const Map<String, IconData> extensionIcons = {
  'extension': Icons.extension_outlined,
  'palette': Icons.palette_outlined,
  'code': Icons.code,
  'build': Icons.build_outlined,
  'memory': Icons.memory,
  'folder': Icons.folder_outlined,
  'terminal': Icons.terminal,
};

/// The store type made of the programs the host lists in [programs]: each store is an item, and + plans its install (or its
/// update, when the host's own copy is there), shows the plan for the person to confirm and runs it.
ExtensionStoreType programsStoreType({
  required String id,
  required String label,
  required IconData icon,
  required ProgramStores programs,
  required String catalog,
}) => ExtensionStoreType(
  id: id,
  label: label,
  icon: icon,
  changes: programs,
  load: () => programs.refresh(latest: true),
  items: () => [
    for (final status in programs.statuses)
      ExtensionStoreItem(
        catalog: catalog,
        name: status.info.title,
        by: status.info.publisher,
        summary: status.installed
            ? 'Installed${status.version == null ? '' : ' ${status.version!}'}${status.hasManaged ? '' : ' (found on this computer)'}'
            : 'Not installed',
        installed: status.installed,
        install: (context) => runProgramAction(context, programs, status, status.hasManaged ? 'update' : 'install'),
      ),
  ],
);

/// The store types beyond the built-in four: Runtimes, and each type an extension named in the host's canoa.cfg adds. The host
/// lists the types and does every install; this only holds what it said.
class StoreExtensions extends ChangeNotifier {
  StoreExtensions({required Future<ControlClient> Function() control, required Future<ControlClient> Function() open})
    : _control = control,
      _open = open {
    _runtimes = ProgramStores(control: control, open: open, kind: 'runtimes');
    _types = [_runtimesType()];
  }

  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;
  late final ProgramStores _runtimes;
  late List<ExtensionStoreType> _types;
  Future<void>? _asking;
  final _held = <ProgramStores>[];

  /// Runtimes first, then what extensions add, in the host's order.
  List<ExtensionStoreType> get types => _types;

  ExtensionStoreType _runtimesType() => programsStoreType(
    id: runtimesTypeId,
    label: runtimesTypeLabel,
    icon: Icons.memory,
    programs: _runtimes,
    catalog: runtimesCatalog,
  );

  /// Asks the host which types extensions add, once. A host that refuses leaves Runtimes alone.
  Future<void> ensure() => _asking ??= _ask();

  Future<void> _ask() async {
    try {
      final client = await _control();
      final answer = await client.call('store/extensions', const {});
      final list = answer is Map && answer['types'] is List ? answer['types'] as List : const [];
      final added = <ExtensionStoreType>[];
      final listing = <Future<void>>[];
      for (final each in list) {
        if (each is! Map || each['id'] is! String || each['title'] is! String) continue;
        final id = each['id'] as String;
        if (id == runtimesTypeId) continue;
        final programs = ProgramStores(control: _control, open: _open, kind: 'extension', type: id);
        added.add(programsStoreType(
          id: id,
          label: each['title'] as String,
          icon: extensionIcons[each['icon']] ?? Icons.extension_outlined,
          programs: programs,
          catalog: each['extension_title'] is String ? each['extension_title'] as String : 'An extension',
        ));
        _held.add(programs);
        listing.add(programs.ensure());
      }
      _types = [_runtimesType(), ...added];
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
    for (final programs in _held) {
      programs.dispose();
    }
    super.dispose();
  }
}
