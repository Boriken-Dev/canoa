/// The Loadouts section's index: New loadout, then the loadouts canoa.cfg names, the default marked, and a search once there are
/// many. Choosing one shows it in the main view; the choice is kept with the shell's layout.
library;

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import '../shell/index_pane.dart';
import '../shell/layout_state.dart';
import 'loadout_editor.dart' show LoadoutDraft;

/// The loadout chosen in the index and the draft being edited, which the index and the main view share.
///
/// The choice is kept with the shell's layout, so it comes back after a restart; one that canoa.cfg no longer holds reads as the
/// default loadout, or the first when there is none.
class LoadoutsSelection extends ChangeNotifier {
  LoadoutsSelection({this.layout});

  /// The section's id in the shell's layout.
  static const section = 'loadouts';

  final ShellLayout? layout;

  /// The loadout being edited, kept here so that it outlives the page: a visit to the Store leaves it as it was.
  final ValueNotifier<LoadoutDraft?> draft = ValueNotifier(null);

  String? _chosen;
  bool _restored = false;

  /// The loadout shown for [config]: the chosen one, else the default, else the first; null when there is none.
  String? selectedIn(CanoaConfig config) {
    if (!_restored) {
      _restored = true;
      _chosen = layout?.of(section).item;
    }
    final names = config.loadouts.keys;
    if (names.contains(_chosen)) return _chosen;
    return config.defaultLoadout ?? names.firstOrNull;
  }

  /// Shows [name]. Leaving a loadout that is being edited drops the edit.
  void select(String name) {
    _chosen = name;
    _restored = true;
    if (draft.value case final editing? when editing.original != name) draft.value = null;
    layout?.setItem(section, name);
    notifyListeners();
  }

  /// Opens the editor on an empty loadout.
  void startNew() {
    draft.value = LoadoutDraft();
    notifyListeners();
  }

  @override
  void dispose() {
    draft.dispose();
    super.dispose();
  }
}

/// More loadouts than this get a search above the list.
const loadoutSearchFrom = 8;

class LoadoutsIndex extends StatefulWidget {
  const LoadoutsIndex({super.key, required this.selection, required this.config});

  final LoadoutsSelection selection;
  final CanoaConfig config;

  @override
  State<LoadoutsIndex> createState() => _LoadoutsIndexState();
}

class _LoadoutsIndexState extends State<LoadoutsIndex> {
  String _query = '';

  /// On a narrow window the index is in the drawer, which gives way to the main view once something is chosen.
  void _closeDrawer() {
    final scaffold = Scaffold.maybeOf(context);
    if (scaffold != null && scaffold.isDrawerOpen) scaffold.closeDrawer();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.selection,
    builder: (context, _) {
      final config = widget.config;
      final chosen = widget.selection.selectedIn(config);
      final scheme = Theme.of(context).colorScheme;
      final shown = [
        for (final MapEntry(:key, :value) in config.loadouts.entries)
          if (_query.isEmpty || key.toLowerCase().contains(_query.toLowerCase()) || value.description.toLowerCase().contains(_query.toLowerCase()))
            (key, value),
      ];
      return IndexPane(
        title: 'Loadouts',
        aboveDivider: [
          InkWell(
            key: const Key('loadout-new'),
            onTap: () {
              widget.selection.startNew();
              _closeDrawer();
            },
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
              child: Row(children: [
                Icon(Icons.add, size: 20, color: scheme.onSurfaceVariant),
                const SizedBox(width: 12),
                const Text('New loadout'),
              ]),
            ),
          ),
          if (config.loadouts.length > loadoutSearchFrom)
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 0, 12, 6),
              child: TextField(
                key: const Key('loadouts-search'),
                decoration: const InputDecoration(isDense: true, prefixIcon: Icon(Icons.search, size: 18), hintText: 'Search loadouts'),
                onChanged: (value) => setState(() => _query = value.trim()),
              ),
            ),
        ],
        body: config.loadouts.isEmpty
            ? const Padding(
                padding: EdgeInsets.all(18),
                child: Text('No loadouts yet. New sessions start with every overlay and every server.', key: Key('loadouts-empty')),
              )
            : ListView(children: [
                for (final (name, loadout) in shown)
                  IndexRow(
                    rowKey: Key('loadouts-index-$name'),
                    leading: Icon(Icons.inventory_2_outlined, size: 20, color: scheme.onSurfaceVariant),
                    titleWidget: Row(children: [
                      Flexible(child: Text(name, maxLines: 1, overflow: TextOverflow.ellipsis)),
                      if (name == config.defaultLoadout)
                        Padding(
                          padding: const EdgeInsets.only(left: 8),
                          child: Text('Default', key: Key('loadouts-index-default-$name'), style: Theme.of(context).textTheme.labelSmall?.copyWith(color: scheme.primary)),
                        ),
                    ]),
                    subtitle: loadoutCounts(loadout),
                    selected: name == chosen,
                    onTap: () {
                      widget.selection.select(name);
                      _closeDrawer();
                    },
                  ),
              ]),
      );
    },
  );
}

/// `2 overlays · 1 skill · 1 MCP server`.
String loadoutCounts(Loadout loadout) {
  String count(int n, String one, String many) => '$n ${n == 1 ? one : many}';
  return '${count(loadout.overlays.length, 'overlay', 'overlays')} · ${count(loadout.skills.length, 'skill', 'skills')} · '
      '${count(loadout.mcp.length, 'MCP server', 'MCP servers')}';
}
