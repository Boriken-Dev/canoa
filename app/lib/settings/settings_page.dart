import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../accounts/accounts_controller.dart';
import '../attention/notifications_host.dart' show NotificationsController;
import '../accounts/accounts_section.dart';
import '../config/canoa_config.dart';
import '../ledger/session_ledger.dart';
import '../providers/provider_catalog.dart';
import '../permissions/permission_rules.dart';
import '../profile/profile_editor.dart' show ProfileChoices;
import '../profile/profile_home.dart';
import '../profile/profiles_panel.dart' show ProfilesPanel;
import '../selectable.dart';
import '../sources/sources.dart';
import '../store/programs.dart';
import '../voice/host_speech.dart';
import '../voice/outputs.dart';
import '../voice/reply_reader.dart';
import '../voice/speech_backend.dart';
import '../voice/voice_settings.dart';
import 'agent_sections.dart';
import 'app_sections.dart';
import '../shell/shell.dart' show ShellScope;
import '../served/portal_session.dart';
import 'notifications_section.dart';
import 'overlay_sections.dart';
import 'sign_in_section.dart';
import 'telemetry_settings.dart';
import 'extension_settings.dart';
import 'provider_switches.dart';
import '../store/catalog_sources.dart';
import '../store/content_settings.dart';
import 'advanced_config.dart';
import 'network_routes.dart';
import 'host_sections.dart';
import 'schedules_settings.dart';
import 'shell_settings.dart';
import 'setting_layers.dart';
import 'settings_controller.dart';

/// One page of Settings in the list at its left.
final class SettingsSection {
  const SettingsSection({required this.id, required this.group, required this.title, required this.icon, required this.keywords, this.host});

  /// The name the page opens at, and the suffix of its list entry's key (`settings-nav-<id>`).
  final String id;
  final String group, title;
  final IconData icon;

  /// What the search box finds the section by, besides its title.
  final String keywords;

  /// What the section's provider says of it, drawn by the generic form renderer when the app has no widget of its own for the id.
  final HostSection? host;

  /// The section a provider describes.
  factory SettingsSection.of(HostSection section) => SettingsSection(
    id: section.id,
    group: section.group ?? defaultSectionGroup,
    title: section.title,
    icon: section.iconData,
    keywords: '${section.keywords} ${section.help ?? ''}',
    host: section,
  );
}

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
    this.hostWorkspacePreview,
    this.onAllowNotifications,
    this.notifications,
    this.profileAgents = const [],
    this.profileChoices,
    this.home,
    this.onPin,
    this.onLoadout,
    this.accounts,
    this.extensions,
    this.providers,
    this.catalogSources,
    this.contentSettings,
    this.advancedConfig,
    this.networkRoutes,
    this.shells,
    this.schedules,
    this.scheduleFolders = const [],
    this.usageSection = false,
    this.voiceBackends = const [],
    this.hostSpeech,
    this.voiceOutputs,
    this.reader,
    this.version = '',
    this.hostSections,
    this.portal,
    this.telemetry,
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

  /// What the host would make of a template; null where there is no host to ask.
  final Future<String?> Function(String template)? hostWorkspacePreview;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  /// What the host says of its events and notifiers, which the Notifications page chooses among; null where there is no host.
  final NotificationsController? notifications;

  /// The agents a profile or an MCP choice is made for.
  final List<String> profileAgents;

  /// What the agent of a source offers before a session starts, for the choices a profile's editor lists.
  final Future<ProfileChoices?> Function(String source)? profileChoices;
  final ProfileHome? home;

  /// Pins the overlay [name] in canoa.cfg to the tree digest just installed, and gives the configuration saved.
  final Future<CanoaConfig?> Function(String name, String digest)? onPin;

  /// Opens the Store.
  final VoidCallback? onLoadout;

  /// The accounts of this computer's host, for the Accounts section; none where the app runs no host.
  final AccountsController? accounts;

  /// The store extensions the host loads, for the Extensions section; none where the key may not change the host's settings.
  final ExtensionSettings? extensions;

  /// The providers the libraries register, each with a switch, for the Providers section; none where the key may not change the
  /// host's settings.
  final ProviderSwitches? providers;

  /// The catalogs of every Store type, with where each comes from and how far it is trusted, for the Catalog sources section; none
  /// where the app has no host.
  final CatalogSources? catalogSources;
  final ContentSettings? contentSettings;

  /// The tables of canoa.cfg no other page edits; null where there is no host to write them.
  final AdvancedConfig? advancedConfig;

  /// The host's network routes, for the Network section; null where there is no host to ask.
  final NetworkRoutes? networkRoutes;

  /// The shells the host found and the choice of which its terminals and agents use, for the Shells section; none where the key may
  /// not change the host's settings.
  final ShellSettings? shells;

  /// The host's scheduled tasks; null where this app has no host to run them or may not set them.
  final ScheduledTasks? schedules;

  /// The folders a task may be offered to run in.
  final List<String> scheduleFolders;

  /// Whether the app has a Usage section of its own, which then holds the usage warning.
  final bool usageSection;

  /// The speech services this build offers for dictation; none makes Voice say so.
  final List<SpeechBackend> voiceBackends;

  /// The speech providers of the host this app reaches, offered beside [voiceBackends].
  final HostSpeechCatalog? hostSpeech;

  /// The voices replies are read aloud with, and what reads a sample of one; Voice offers their choices when given.
  final VoiceOutputs? voiceOutputs;
  final ReplyReader? reader;

  /// The app's version, for About.
  final String version;

  /// The sections the host's providers offer: each is a section of its own, or a group inside one of the sections here.
  final HostSections? hostSections;

  /// The portal that served this page, whose own pages the Sign-in section opens; none for a page served by anything else.
  final PortalSession? portal;

  /// Telemetry export: what the host records, the switch and the exporters; none where the key may not change the host's own settings.
  final TelemetrySettings? telemetry;

  @override
  State<SettingsPage> createState() => SettingsPageState();
}

class SettingsPageState extends State<SettingsPage> {
  late final SettingsController controller = SettingsController(config: widget.config, apply: widget.apply)..addListener(_changed);
  final _search = TextEditingController();
  String? _selected;

  /// The sections of a page given none: those of an app with no host to ask.
  late final HostSections _none = HostSections.withoutHost();
  HostSections get _held => widget.hostSections ?? _none;

  /// The widget the app draws for the section [id] itself, with whether this build and key have what it needs. A section the app draws
  /// is listed by the host like any other and the widget is only chosen here, by its id; an id with no entry is drawn from the form
  /// its provider describes.
  Map<String, ({bool ready, Widget Function() build})> get _own => {
    'general': (ready: true, build: () => GeneralSection(controller: controller, ledger: widget.ledger, hostWorkspacePreview: widget.hostWorkspacePreview)),
    'appearance': (ready: true, build: () => AppearanceSection(controller: controller)),
    'notifications': (ready: true, build: () => NotificationsSection(key: const Key('settings-notifications'), controller: widget.notifications, onAllowNotifications: widget.onAllowNotifications)),
    'voice': (
      ready: true,
      build: () => VoiceSection(controller: controller, backends: widget.voiceBackends, host: widget.hostSpeech, outputs: widget.voiceOutputs, reader: widget.reader),
    ),
    'shortcuts': (ready: true, build: () => ShortcutsSection(controller: controller)),
    'sources': (
      ready: true,
      build: () => SourcesSection(
        controller: controller, presets: widget.presets, hosted: widget.hosted, checkSource: widget.checkSource, programs: widget.programs, accounts: widget.accounts),
    ),
    'profiles': (
      ready: true,
      build: () => ProfilesPanel(
        key: const Key('settings-profiles-panel'),
        config: controller.config,
        agents: widget.profileAgents,
        choicesFor: widget.profileChoices,
        home: widget.home,
        onPin: widget.onPin,
        onLoadout: widget.onLoadout,
        onChange: controller.change,
      ),
    ),
    'mcp': (ready: true, build: () => McpSection(controller: controller, agents: widget.profileAgents, onLoadout: widget.onLoadout)),
    'permissions': (ready: true, build: () => PermissionsSection(controller: controller, rules: widget.rules, sources: widget.profileAgents)),
    'accounts': (ready: widget.accounts != null, build: () => AccountsSection(controller: widget.accounts!)),
    'shells': (ready: widget.shells != null, build: () => ShellsSection(key: const Key('settings-shells'), settings: widget.shells!)),
    'schedules': (ready: widget.schedules != null, build: () => SchedulesSection(key: const Key('settings-schedules'), tasks: widget.schedules!, folders: widget.scheduleFolders)),
    'usage': (ready: !widget.usageSection, build: () => UsageWarning(controller: controller)),
    'extensions': (ready: widget.extensions != null, build: () => ExtensionsSection(key: const Key('settings-extensions'), settings: widget.extensions!)),
    'catalog_sources': (ready: widget.catalogSources != null, build: () => CatalogSourcesSection(key: const Key('settings-catalog-sources'), sources: widget.catalogSources!)),
    'content': (ready: widget.contentSettings != null, build: () => ContentSection(key: const Key('settings-content'), content: widget.contentSettings!)),
    'network': (ready: widget.networkRoutes != null, build: () => NetworkSection(key: const Key('settings-network-section'), routes: widget.networkRoutes!)),
    'advanced': (ready: widget.advancedConfig != null, build: () => AdvancedSection(key: const Key('settings-advanced'), config: widget.advancedConfig!)),
    'providers': (ready: widget.providers != null, build: () => ProvidersSection(key: const Key('settings-providers'), switches: widget.providers!)),
    'telemetry': (ready: widget.telemetry != null, build: () => TelemetrySection(key: const Key('settings-telemetry'), settings: widget.telemetry!)),
    'signin': (ready: widget.portal != null, build: () => SignInSection(key: const Key('settings-signin'), portal: widget.portal!)),
    'about': (ready: true, build: () => AboutSection(version: widget.version, host: widget.hostSpeech)),
  };

  /// The sections the host lists, in its order but with About last: those the app draws that this build and key have, and the
  /// provider's own. A section that is a group inside another one is not a page of its own.
  List<SettingsSection> get _available {
    final held = _held;
    final known = {for (final section in held.sections) if (section.under == null) section.id};
    final pages = <SettingsSection>[];
    final taken = <String>{};
    for (final section in held.ownOf(known)) {
      final own = _own[section.id];
      // An id the app draws itself belongs to the page the host registers for it or one of the libraries it carries, whoever else
      // lists one.
      if (own != null && (!own.ready || (section.origin != 'built-in' && section.origin != 'bundled'))) continue;
      if (taken.add(section.id)) pages.add(SettingsSection.of(section));
    }
    // The page of the portal that served this one: only that portal knows what it keeps, so the host does not list it.
    if (widget.portal != null && taken.add('signin')) pages.add(_signIn);
    return [...pages.where((section) => section.id != 'about'), ...pages.where((section) => section.id == 'about')];
  }

  static const _signIn = SettingsSection(
    id: 'signin',
    group: 'App',
    title: '', // named by the text 'settings.section.signin'
    icon: Icons.password,
    keywords: 'one-time code otp authenticator totp passkey certificate sign out password portal account',
  );

  /// The groups providers put inside the section [id].
  List<HostSection> _groups(String id) => _held.groupsIn(id);

  @override
  void initState() {
    super.initState();
    _held.addListener(_changed);
    if (_available.any((section) => section.id == widget.section)) _selected = widget.section;
  }

  @override
  void didUpdateWidget(SettingsPage oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.hostSections != widget.hostSections) {
      (oldWidget.hostSections ?? _none).removeListener(_changed);
      _held.addListener(_changed);
    }
  }

  void _changed() {
    if (!mounted) return;
    // The page can open before the host has listed the sections: the one it was asked to show is chosen once it is there.
    if (_selected == null && widget.section != null && _available.any((section) => section.id == widget.section)) _selected = widget.section;
    setState(() {});
  }

  /// Shows the section [id], as when the page is already open and something sends the person to it.
  void show(String id) {
    if (_available.any((section) => section.id == id)) setState(() => _selected = id);
  }

  @override
  void dispose() {
    _held.removeListener(_changed);
    controller
      ..removeListener(_changed)
      ..dispose();
    _search.dispose();
    super.dispose();
  }

  /// The names of the loaded agents, which the sections about agents are also found by.
  String _agentWords(SettingsSection section) =>
      section.id == 'sources' || section.id == 'accounts' ? providerCatalog.providers.map((each) => '${each.title} ${each.program}').join(' ') : '';

  /// What the groups inside a section are called and found by, so a search for one finds the section that holds it.
  String _groupWords(SettingsSection section) => _groups(section.id).map((group) => '${group.title} ${group.keywords}').join(' ');

  List<SettingsSection> get _matching {
    final words = _search.text.toLowerCase().split(' ').where((word) => word.isNotEmpty);
    return [
      for (final section in _available)
        if (words.every((word) => '${section.title} ${section.keywords} ${_agentWords(section)} ${_groupWords(section)}'.toLowerCase().contains(word))) section,
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
          decoration: InputDecoration(prefixIcon: const Icon(Icons.search), hintText: context.t('settings.search'), isDense: true, border: const OutlineInputBorder()),
          onChanged: (_) => setState(() {}),
        ),
      ),
      Expanded(
        child: ListView(padding: const EdgeInsets.only(bottom: 16), children: [
          if (matching.isEmpty) Padding(padding: const EdgeInsets.all(16), child: Text(context.t('settings.no_match'), key: const Key('settings-no-match'))),
          for (final group in groups) ...[
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
              child: Text(AppText.of(context).maybe('settings.group.${group.toLowerCase()}') ?? group, style: Theme.of(context).textTheme.labelMedium?.copyWith(color: scheme.onSurfaceVariant)),
            ),
            for (final section in matching.where((section) => section.group == group))
              ListTile(
                key: Key('settings-nav-${section.id}'),
                dense: true,
                leading: Icon(section.icon, size: 20),
                title: Text(_titleOf(context, section)),
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

  /// What a section is called: the app's own word for it where there is one, else what its provider says.
  String _titleOf(BuildContext context, SettingsSection section) => AppText.of(context).maybe('settings.section.${section.id}') ?? section.title;

  /// The section shown: the one picked if the search still lists it, else the first listed.
  SettingsSection? _current(List<SettingsSection> matching) =>
      matching.where((section) => section.id == _selected).firstOrNull ?? matching.firstOrNull;

  Widget _body(BuildContext context, SettingsSection section) {
    final child = _own[section.id]?.build() ?? HostSectionForm(key: Key('settings-host-${section.id}'), section: section.host!, sections: _held);
    final groups = _groups(section.id);
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
              child: Text(_titleOf(context, section), style: Theme.of(context).textTheme.headlineSmall),
            ),
            if (controller.problem case final problem?)
              Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: Text(problem, key: const Key('settings-write-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
            SettingsLayers(controller: controller, child: child),
            for (final group in groups) HostSectionForm(key: Key('settings-host-${group.id}'), section: group, sections: _held, asGroup: true),
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
            title: Text(inSection ? _titleOf(context, current) : context.t('nav.settings')),
            automaticallyImplyLeading: false,
            leading: inSection
                ? IconButton(
                    key: const Key('settings-back'),
                    icon: const Icon(Icons.arrow_back),
                    tooltip: 'Back to the list',
                    onPressed: () => setState(() => _selected = null),
                  )
                : ShellScope.menuButton(context),
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
