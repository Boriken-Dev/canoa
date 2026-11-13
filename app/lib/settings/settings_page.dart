import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../desktop/host_supervisor.dart';
import '../desktop/hosting_panel.dart' show HostingPanel, SaveHosting;
import '../desktop/host_tool.dart' show freeLoopbackPort;
import '../ledger/session_ledger.dart';
import '../permissions/permission_rules.dart';
import '../profile/profile_home.dart';
import '../profile/profiles_panel.dart' show ProfilesPanel;
import '../selectable.dart';
import '../sources/sources.dart';
import '../store/programs.dart';
import 'agent_sections.dart';
import 'app_sections.dart';
import 'overlay_sections.dart';
import 'settings_controller.dart';

/// One page of Settings in the list at its left.
final class SettingsSection {
  const SettingsSection({required this.id, required this.group, required this.title, required this.icon, required this.keywords});

  /// The name the page opens at, and the suffix of its list entry's key (`settings-nav-<id>`).
  final String id;
  final String group, title;
  final IconData icon;

  /// What the search box finds the section by, besides its title.
  final String keywords;
}

const _sections = [
  SettingsSection(id: 'general', group: 'App', title: 'General', icon: Icons.tune, keywords: 'tray window startup network webtransport timeout sessions remembered folder workspace keep history'),
  SettingsSection(id: 'appearance', group: 'App', title: 'Appearance', icon: Icons.palette_outlined, keywords: 'look theme brightness dark light detail verbose density preview'),
  SettingsSection(id: 'notifications', group: 'App', title: 'Notifications', icon: Icons.notifications_none, keywords: 'alerts permission question finished browser toast'),
  SettingsSection(id: 'shortcuts', group: 'App', title: 'Shortcuts', icon: Icons.keyboard_outlined, keywords: 'keyboard keys chords hotkeys'),
  SettingsSection(id: 'sources', group: 'Agents', title: 'Sources', icon: Icons.hub_outlined, keywords: 'agents providers programs claude codex command install update version ready'),
  SettingsSection(id: 'profiles', group: 'Agents', title: 'Profiles', icon: Icons.layers_outlined, keywords: 'overlays layers model effort mode skills trusted directories project'),
  SettingsSection(id: 'mcp', group: 'Agents', title: 'MCP servers', icon: Icons.extension_outlined, keywords: 'tools servers mcp allowed per agent'),
  SettingsSection(id: 'permissions', group: 'Agents', title: 'Permissions', icon: Icons.shield_outlined, keywords: 'bypass mode allow once remembered rules protected paths log revoke full access'),
  SettingsSection(id: 'usage', group: 'Agents', title: 'Usage', icon: Icons.speed, keywords: 'limit warn percent quota'),
  SettingsSection(id: 'hosting', group: 'Agents', title: 'Hosting', icon: Icons.dns_outlined, keywords: 'serve browsers keys pair pin host listen'),
  SettingsSection(id: 'catalogs', group: 'Overlays', title: 'Catalogs', icon: Icons.storefront_outlined, keywords: 'store limits catalog sources icons size'),
  SettingsSection(id: 'about', group: 'About', title: 'About', icon: Icons.info_outline, keywords: 'version canoa boriken licence'),
];

/// Settings as a page: a list of sections at the left (a page of its own at phone width), a search over their keywords, and
/// rows that apply as they change. The configuration is written through [apply] on every change; there is nothing to save.
class SettingsPage extends StatefulWidget {
  const SettingsPage({
    super.key,
    required this.config,
    required this.apply,
    this.section,
    this.ledger,
    this.rules,
    this.presets = const [],
    this.hosted = const [],
    this.checkSource,
    this.programs,
    this.workspacePreview,
    this.onAllowNotifications,
    this.profileAgents = const [],
    this.home,
    this.onPin,
    this.onLoadout,
    this.supervisor,
    this.saveHosting,
    this.freePort = freeLoopbackPort,
    this.version = '',
  });

  final CanoaConfig config;
  final ApplyConfig apply;

  /// The section the page opens at; the first when null or unknown.
  final String? section;

  /// The session list, whose kept text Settings can clear.
  final SessionLedger? ledger;

  /// What the person told Canoa to allow without asking, which Settings lists and revokes at once.
  final PermissionRules? rules;

  /// The built-in sources, which can be hidden from a new session but not changed.
  final List<AgentSource> presets;

  /// The agent entries of the canoa this app runs, which Hosting changes.
  final List<String> hosted;

  /// What a source needs and whether it is here.
  final Future<SourceCheck> Function(AgentSource source)? checkSource;

  /// The programs the Store reports, which the sources show the version of and install or update.
  final ProgramStores? programs;

  /// The folder a template makes for a session, as the app would make it now; null where this build has none.
  final String? Function(String template)? workspacePreview;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  /// The agents a profile or an MCP choice is made for.
  final List<String> profileAgents;
  final ProfileHome? home;

  /// Pins the overlay [name] in canoa.cfg to the tree digest just installed, and gives the configuration saved.
  final Future<CanoaConfig?> Function(String name, String digest)? onPin;

  /// Opens Loadout.
  final VoidCallback? onLoadout;

  /// The host this app runs, for the Hosting section; none where the app runs none.
  final HostSupervisor? supervisor;
  final SaveHosting? saveHosting;
  final Future<int> Function() freePort;

  /// The app's version, for About.
  final String version;

  @override
  State<SettingsPage> createState() => SettingsPageState();
}

class SettingsPageState extends State<SettingsPage> {
  late final SettingsController controller = SettingsController(config: widget.config, apply: widget.apply)..addListener(_changed);
  final _search = TextEditingController();
  String? _selected;

  List<SettingsSection> get _available => [
    for (final section in _sections)
      if (section.id != 'hosting' || widget.supervisor != null) section,
  ];

  @override
  void initState() {
    super.initState();
    if (_available.any((section) => section.id == widget.section)) _selected = widget.section;
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    controller
      ..removeListener(_changed)
      ..dispose();
    _search.dispose();
    super.dispose();
  }

  List<SettingsSection> get _matching {
    final words = _search.text.toLowerCase().split(' ').where((word) => word.isNotEmpty);
    return [
      for (final section in _available)
        if (words.every((word) => '${section.title} ${section.keywords}'.toLowerCase().contains(word))) section,
    ];
  }

  Widget _nav(BuildContext context, {required bool wide}) {
    final matching = _matching;
    final scheme = Theme.of(context).colorScheme;
    final groups = {for (final section in matching) section.group};
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: const EdgeInsets.all(12),
        child: TextField(
          key: const Key('settings-search'),
          controller: _search,
          decoration: const InputDecoration(prefixIcon: Icon(Icons.search), hintText: 'Search settings', isDense: true, border: OutlineInputBorder()),
          onChanged: (_) => setState(() {}),
        ),
      ),
      Expanded(
        child: ListView(padding: const EdgeInsets.only(bottom: 16), children: [
          if (matching.isEmpty) const Padding(padding: EdgeInsets.all(16), child: Text('No setting matches.', key: Key('settings-no-match'))),
          for (final group in groups) ...[
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
              child: Text(group, style: Theme.of(context).textTheme.labelMedium?.copyWith(color: scheme.onSurfaceVariant)),
            ),
            for (final section in matching.where((section) => section.group == group))
              ListTile(
                key: Key('settings-nav-${section.id}'),
                dense: true,
                leading: Icon(section.icon, size: 20),
                title: Text(section.title),
                selected: wide && _current(matching)?.id == section.id,
                selectedTileColor: scheme.surfaceContainerHighest,
                shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
                contentPadding: const EdgeInsets.symmetric(horizontal: 12),
                onTap: () => setState(() => _selected = section.id),
              ),
          ],
        ]),
      ),
    ]);
  }

  /// The section shown: the one picked if the search still lists it, else the first listed.
  SettingsSection? _current(List<SettingsSection> matching) =>
      matching.where((section) => section.id == _selected).firstOrNull ?? matching.firstOrNull;

  Widget _body(BuildContext context, SettingsSection section) {
    final config = controller.config;
    final child = switch (section.id) {
      'general' => GeneralSection(controller: controller, ledger: widget.ledger, workspacePreview: widget.workspacePreview),
      'appearance' => AppearanceSection(controller: controller),
      'notifications' => NotificationsSection(controller: controller, onAllowNotifications: widget.onAllowNotifications),
      'shortcuts' => const ShortcutsSection(),
      'sources' => SourcesSection(
        controller: controller, presets: widget.presets, hosted: widget.hosted, checkSource: widget.checkSource, programs: widget.programs),
      'profiles' => ProfilesPanel(
        key: const Key('settings-profiles-panel'),
        config: config,
        agents: widget.profileAgents,
        home: widget.home,
        onPin: widget.onPin,
        onLoadout: widget.onLoadout,
      ),
      'mcp' => McpSection(controller: controller, agents: widget.profileAgents, onLoadout: widget.onLoadout),
      'permissions' => PermissionsSection(controller: controller, rules: widget.rules, sources: widget.profileAgents),
      'usage' => UsageSection(controller: controller),
      'hosting' => HostingPanel(
        key: const Key('settings-hosting-panel'),
        supervisor: widget.supervisor!,
        config: config,
        onSave: widget.saveHosting!,
        freePort: widget.freePort,
      ),
      'catalogs' => CatalogsSection(controller: controller),
      _ => AboutSection(version: widget.version),
    };
    return Align(
      alignment: Alignment.topCenter,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 760),
        child: ListView(
          key: Key('settings-section-${section.id}'),
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
          children: [
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(section.title, style: Theme.of(context).textTheme.headlineSmall),
            ),
            if (controller.problem case final problem?)
              Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: Text(problem, key: const Key('settings-write-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
            child,
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(builder: (context, constraints) {
    final wide = constraints.maxWidth >= 760;
    final matching = _matching;
    final current = _current(matching);
    // At phone width the list is a page and a section another: back goes from one to the list.
    final inSection = !wide && _selected != null && current != null;
    return PopScope(
      canPop: !inSection,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) setState(() => _selected = null);
      },
      child: Selectable(
        child: Scaffold(
          appBar: AppBar(
            title: Text(inSection ? current.title : 'Settings'),
            leading: inSection
                ? IconButton(
                    key: const Key('settings-back'),
                    icon: const Icon(Icons.arrow_back),
                    tooltip: 'Back to the list',
                    onPressed: () => setState(() => _selected = null),
                  )
                : null,
          ),
          body: wide
              ? Row(children: [
                  SizedBox(width: 260, child: _nav(context, wide: true)),
                  const VerticalDivider(width: 1),
                  Expanded(child: current == null ? const SizedBox() : _body(context, current)),
                ])
              : inSection
              ? _body(context, current)
              : _nav(context, wide: false),
        ),
      ),
    );
  });
}

/// Opens Settings as a page over the app.
Future<void> showSettings(BuildContext context, SettingsPage page) =>
    Navigator.of(context).push<void>(MaterialPageRoute(builder: (_) => page));
