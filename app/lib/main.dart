import 'attention/notifier.dart';
import 'config/settings_dialog.dart';
import 'dart:async';
import 'dart:convert' show jsonEncode;
import 'config/canoa_config.dart';
import 'config/canoa_config_file.dart';
import 'desktop/desktop_shell.dart';
import 'desktop/host_control.dart';
import 'desktop/host_supervisor.dart';
import 'desktop/host_tool.dart';
import 'desktop/hosting_dialog.dart';
import 'sources/source_choices.dart';
import 'extension/composer_extension.dart';
import 'extension/site_documents.dart';
import 'extension/site_sessions.dart';
import 'extension/tab_bridge.dart' show TabBridge;
import 'extension/tab_shares.dart' show TabShares;
import 'extension_context.dart';
import 'acp/agent_commands.dart' show AgentCommand;
import 'acp/browser_connect.dart';
import 'acp/connect.dart' show chatEndpoints;
import 'acp/pipe_channel.dart' show isPipeUrl;
import 'acp/provider_connect.dart';
import 'acp/client.dart';
import 'acp/local_agent.dart';
import 'desktop/child_flag.dart';
import 'package:coqui_acp/page.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'connect_dialog.dart';
import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';

import 'chat/cache_lifetime.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'chat/clipboard.dart';
import 'chat/composer.dart';
import 'chat/new_session_view.dart';
import 'chat/palette.dart';
import 'chat/agent_events.dart' show AgentCommandInfo;
import 'chat/session_views.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'chat/update_log.dart';
import 'auth/peer_proof.dart' show PeerMethod;
import 'history/session_move.dart';
import 'ledger/host_ledger_backend.dart';
import 'ledger/ledger_backend.dart';
import 'ledger/session_ledger.dart';
import 'permissions/permission_rules.dart';
import 'popout/platform.dart' as popout_platform;
import 'popout/popout_app.dart';
import 'popout/popout_hub.dart';
import 'permissions/protected_paths.dart' show ProtectedPaths;
import 'permissions/real_path.dart';
import 'permissions/rule_log.dart';
import 'profile/profile_home_platform.dart';
import 'acp/usage_wire.dart' show ProfileApprover, noApproval;
import 'profile/approval_dialog.dart';
import 'profile/report.dart';
import 'profile/profiles_dialog.dart';
import 'rail/session_groups.dart';
import 'rail/session_rail.dart';
import 'session_store.dart';
import 'sources/readiness.dart';
import 'sources/sources.dart';
import 'loadout/loadout_page.dart';
import 'loadout/registry_fetch.dart';
import 'store/programs.dart';
import 'welcome/welcome_view.dart';
import 'drafts/draft_backend.dart';
import 'drafts/draft_store.dart';
import 'theme/look.dart';
import 'auth/browser_peer.dart' show verifyBrowserPeer;
import 'workspace/documents.dart' show hasWorkspaceFolders, homeFolder, makeFolder, removeEmptyFolder;
import 'workspace/scratch.dart';
import 'workspace/workspace_template.dart' show exampleSessionId;
import 'worktree/close_dialog.dart';
import 'worktree/host.dart';
import 'worktree/worktree.dart';
import 'selectable.dart';

/// The folder [file] is in, else null.
String? _parentFolder(String? file) {
  if (file == null) return null;
  final text = file.replaceAll('\\', '/');
  final cut = text.lastIndexOf('/');
  return cut <= 0 ? null : text.substring(0, cut);
}

Future<void> main(List<String> args) async {
  // The executable's own entry takes this before Flutter starts; reaching here means a runner built without it, which must
  // not answer a host's own command by opening another window.
  if (args.firstOrNull == canoaChildFlag) throw StateError('this build has no entry for $canoaChildFlag');
  // A window the app opened for one conversation runs this same program, and only shows that conversation.
  final popped = popout_platform.popoutLaunch(args);
  if (popped != null) return runPopout(popped);
  WidgetsFlutterBinding.ensureInitialized();
  final config = await loadConfig();
  final shell = createDesktopShell();
  final host = shell.isDesktop
      ? HostSupervisor(findHostTool(), off: !config.localHost || args.contains('--no-local-host'))
      : null;
  // The host starts first: it keeps the session list, which it answers once it is up.
  final hosting = host?.apply(configPath: desktopConfigPath());
  // In the host's home on a desktop that runs one, else beside canoa.cfg, or in this origin's IndexedDB on the web.
  final keptByHost = host != null && host.phase != HostPhase.off && host.phase != HostPhase.absent;
  final ledger = SessionLedger(
    keptByHost ? HostLedgerBackend(host) : platformLedgerBackend(configPath: desktopConfigPath()),
    settings: config.sessions,
  );
  await ledger.load();
  final configFile = desktopConfigPath();
  final rules = PermissionRules(
    platformLedgerBackend(configPath: configFile, name: 'permissions'),
    log: platformRuleLog(configPath: configFile),
    guards: PermissionGuards(
      home: homeFolder(),
      config: _parentFolder(configFile),
      protected: config.protectedPaths == null ? null : ProtectedPaths(config.protectedPaths!),
    ),
    real: platformRealPath,
    repoRoot: platformRepoRoot,
  );
  await rules.load();
  final drafts = DraftKeeper(platformDraftBackend(configPath: desktopConfigPath()));
  final settingsSignal = ValueNotifier<int>(0);
  final hostingSignal = ValueNotifier<int>(0);
  await shell.initialize(
    config,
    onSettings: () => settingsSignal.value++,
    onHosting: host == null ? null : () => hostingSignal.value++,
  );
  runApp(CanoaApp(
    config: config, shell: shell, settingsSignal: settingsSignal,
    host: host, hostingSignal: hostingSignal, ledger: ledger, rules: rules, notifier: platformNotifier(), drafts: drafts,
  ));
  await hosting;
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal, this.host, this.hostingSignal,
    this.configPath, this.writeConfig, this.ledger, this.rules, this.notifier, this.drafts,
    this.isGitRepository, this.folderExists, this.pickFolder, this.pickFiles, this.statFile, this.clipboard,
    this.worktrees, this.documents, this.askChoices, this.renderers = chat.ChatRenderers.none, this.popoutWindows,
    this.tabBridge, this.browserProvider,
  });

  /// How a conversation is shown in a window of its own: the platform's by default, and a test's own.
  final PopoutWindows? popoutWindows;

  /// The extension's tabs and the browser's provider of agents, by default the platform's; a test gives its own.
  @visibleForTesting
  final TabBridge? tabBridge;
  @visibleForTesting
  final Future<AcpPage?>? browserProvider;

  /// What the conversation draws beyond the package's own: Canoa's content kinds (a delegation card, a chart), a card for a kind of
  /// tool, the lines a burst or a turn folds into. Registered here, never in the transcript.
  final chat.ChatRenderers renderers;

  /// Where sessions not yet started are kept between runs, for a store this app makes.
  final DraftKeeper? drafts;

  /// The folder, file and clipboard services the composer uses, each the platform's by default.
  @visibleForTesting
  final bool Function(String folder)? isGitRepository;
  @visibleForTesting
  final bool Function(String folder)? folderExists;
  @visibleForTesting
  final Future<String?> Function()? pickFolder;
  @visibleForTesting
  final Future<List<XFile>> Function()? pickFiles;
  @visibleForTesting
  final Future<int?> Function(String path)? statFile;
  @visibleForTesting
  final chat.ClipboardReader? clipboard;

  /// The Documents folder `${documents}` stands for; by default the system's.
  @visibleForTesting
  final String? documents;

  /// Makes worktrees for agents on this computer; by default through this app's host.
  @visibleForTesting
  final Worktrees? Function()? worktrees;

  /// Asks for a source's models and efforts before its agent runs; by default the host this app carries is asked.
  @visibleForTesting
  final ChoicesLoader? askChoices;

  final SessionStore? store;

  /// How a session that needs the person reaches them outside the window; none, nothing does.
  final Notifier? notifier;

  /// The list of sessions kept across runs, for a store this app makes; a given [store] brings its own.
  final SessionLedger? ledger;

  /// What the person told Canoa to allow without asking, kept across runs, for a store this app makes.
  final PermissionRules? rules;
  final CanoaConfig config;
  final DesktopShell? shell;
  final ValueNotifier<int>? settingsSignal;

  /// This app's own host, on a desktop.
  final HostSupervisor? host;
  final ValueNotifier<int>? hostingSignal;

  /// Where canoa.cfg is and how it is written; by default the platform's place.
  @visibleForTesting
  final String? configPath;
  @visibleForTesting
  final Future<void> Function(CanoaConfig)? writeConfig;

  @override
  State<CanoaApp> createState() => _CanoaAppState();
}

class _CanoaAppState extends State<CanoaApp> {
  late final SessionStore store;
  late final bool ownsStore;
  late CanoaConfig config;
  // The tray asks between frames, and a frame may not come by itself while the window is hidden.
  void _requestSettings() => scheduleMicrotask(() {
    if (mounted) unawaited(_showSettings());
  });
  void _requestHosting() => scheduleMicrotask(() {
    if (mounted) unawaited(_showHosting());
  });

  String? get _configPath => widget.configPath ?? desktopConfigPath();
  Future<void> _write(CanoaConfig changed) => (widget.writeConfig ?? saveConfig)(changed);

  // One of each at a time: a second copy would hold a configuration the first one changes.
  bool _hostingOpen = false, _settingsOpen = false;

  /// The look and brightness being tried in Settings, which are the app's until it closes; never written.
  CanoaConfig? _preview;

  Future<void> _showHosting() async {
    final host = widget.host;
    if (host == null || _hostingOpen) return;
    final above = _navigator.currentContext;
    if (above == null) return;
    _hostingOpen = true;
    try {
      await showSelectableDialog<void>(
        context: above,
        builder: (_) => HostingDialog(supervisor: host, config: config, onSave: _saveHosting),
      );
    } finally {
      _hostingOpen = false;
    }
  }

  /// Saves canoa.cfg once canoa finds nothing wrong with it, then serves it.
  /// Only [host] comes from the dialog: the rest is what the app has now, which Settings may have changed meanwhile.
  Future<String?> _saveHosting(CanoaConfig fromDialog) async {
    final host = widget.host;
    final path = _configPath;
    if (host == null || path == null) return 'canoa.cfg has no place on this computer';
    final changed = config.withHost(fromDialog.host);
    if (changed.hostEntries.isNotEmpty) {
      final problem = await withFileBeside(path, changed.encode(), host.check);
      if (problem != null) return problem;
    }
    await _write(changed);
    if (mounted) setState(() => config = changed);
    await host.apply(configPath: path);
    return null;
  }

  Future<void> _showSettings({bool sources = false}) async {
    final shell = widget.shell;
    final above = _navigator.currentContext;
    if (above == null || _settingsOpen) return;
    _settingsOpen = true;
    final CanoaConfig? fromDialog;
    try {
      fromDialog = await showSelectableDialog<CanoaConfig>(
        context: above,
        builder: (_) => SettingsDialog(
          config: config, onQuit: shell != null && shell.isDesktop ? shell.quit : null, ledger: store.ledger, rules: store.rules,
          onProfiles: _showProfiles,
          onPreview: (preview) => setState(() => _preview = preview),
          workspacePreview: hasWorkspaceFolders && canRunLocalAgents
              ? (template) => scratchFolder(template, exampleSessionId, source: _lastSource, documents: widget.documents)
              : null,
          onAllowNotifications: kIsWeb ? widget.notifier?.requestPermission : null,
          presets: [
            for (final source in _hostedSources)
              if (source.local && !config.agents.any((agent) => agent.label == source.label)) source,
          ],
          hosted: [for (final source in _hostedSources) if (!source.local) source.hostEntry!],
          checkSource: _check,
          programs: _programs,
          openSources: sources,
        ),
      );
    } finally {
      _settingsOpen = false;
      // However Settings closed, the look it previewed goes; a save sets the chosen one in the same frame, below.
      if (_preview != null && mounted) setState(() => _preview = null);
    }
    if (fromDialog == null || !mounted) return;
    // [host] and the profile tables stay as the app has them now: Hosting may have changed one, and an overlay installed
    // from Profiles is pinned in [overlays], while Settings was open.
    var changed = fromDialog.withHost(config.host).withProfilesOf(config);
    // The host serves a source from the moment it starts, so adding or removing one restarts it, which ends the sessions open
    // on any of its sources.
    // Profiles are read by the host when it starts too, so a change to them restarts it as well.
    final sameProfiles = _sameProfiles(changed, config);
    var restart = widget.host != null && (!_sameAgents(changed.agents, config.agents) || !sameProfiles);
    if (restart && _openOnHost() > 0 && !await _confirmRestart()) {
      changed = changed.copyWith(agents: config.agents);
      if (!sameProfiles) changed = changed.withProfilesOf(config);
      restart = false;
    }
    if (!mounted) return;
    setState(() => config = changed);
    store.embedLimit = changed.embedLimit;
    if (_checked) unawaited(_checkSources());
    store.ledger?.configure(changed.sessions);
    store.defaultMode = changed.defaultMode;
    store.bypassOnceOnly = changed.allowOnceOnly;
    await widget.shell?.update(changed);
    try {
      await _write(changed);
      if (restart && _configPath != null) await widget.host!.apply(configPath: _configPath);
    } on UnsupportedError {
      // canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      if (mounted) {
        _messenger.currentState?.showSnackBar(
          SnackBar(content: Text('Could not save canoa.cfg: $error')),
        );
      }
    }
  }

  /// Whether `[profile]`, `[overlays]` and the `[agents]` tables (a profile and MCP choices live in them) read the same.
  static bool _sameProfiles(CanoaConfig a, CanoaConfig b) =>
      const ['profile', 'overlays', 'agents'].every((key) => jsonEncode(a.tables[key]) == jsonEncode(b.tables[key]));

  static bool _sameAgents(List<AgentCommand> a, List<AgentCommand> b) =>
      a.length == b.length &&
      [for (var i = 0; i < a.length; i++) a[i].label == b[i].label && a[i].program == b[i].program && _sameWords(a[i].args, b[i].args)]
          .every((same) => same);

  static bool _sameWords(List<String> a, List<String> b) =>
      a.length == b.length && [for (var i = 0; i < a.length; i++) a[i] == b[i]].every((same) => same);

  /// How many sessions are open on this computer's own host, which restarting it ends.
  int _openOnHost() => store.sessions
      .where((session) => session.place == SessionPlace.local && session.status != SessionStatus.draft && session.status != SessionStatus.disconnected)
      .length;

  Future<bool> _confirmRestart() async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return false;
    final count = _openOnHost();
    final ok = await showSelectableDialog<bool>(
      context: above,
      builder: (context) => AlertDialog(
        key: const Key('restart-host'),
        title: const Text('Restart the host?'),
        content: Text('A source added or removed takes effect when Canoa\'s host restarts, and that ends the '
            '${count == 1 ? 'session' : '$count sessions'} open on this computer now. Their conversations stay in the list '
            'to open again.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep the sources as they were')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Restart')),
        ],
      ),
    );
    return ok ?? false;
  }

  /// The agents the app offers a profile for: the host's own sources, the commands in canoa.cfg and any agent with a profile.
  List<String> get _profileAgents => {
    for (final source in _hostedSources) if (source.local) source.label,
    for (final agent in config.agents) agent.label,
    ...config.agentProfiles.keys,
  }.toList();

  /// Loadout: overlays, skills and MCP servers, installed and to discover.
  Future<void> _showLoadout() async {
    final above = _navigator.currentContext;
    if (above == null) return;
    final left = await showLoadout(
      above,
      config: config,
      home: platformProfileHome(configPath: _configPath),
      onSave: _saveLoadout,
      agents: _profileAgents,
      programs: _programs,
      fetchRegistry: kIsWeb ? null : fetchRegistry,
      findProgram: findProgram,
    );
    if (left != null && mounted) setState(() => config = left);
  }

  /// Saves what Loadout changed. The host reads the profile when it starts, so a change to it is applied by starting the host
  /// again, which ends the sessions open on it: with some open, the person is asked, and may leave it for the next start.
  Future<CanoaConfig?> _saveLoadout(CanoaConfig changed) async {
    var restart = widget.host != null && _configPath != null && !_sameProfiles(changed, config);
    if (restart && _openOnHost() > 0 && !await _confirmApplyProfile()) restart = false;
    await _write(changed);
    if (mounted) setState(() => config = changed);
    if (restart) await widget.host!.apply(configPath: _configPath);
    return changed;
  }

  Future<bool> _confirmApplyProfile() async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return false;
    final count = _openOnHost();
    final ok = await showSelectableDialog<bool>(
      context: above,
      builder: (context) => AlertDialog(
        key: const Key('apply-profile'),
        title: const Text('Apply it now?'),
        content: Text('A new session starts with the change once the host starts again, and that ends the '
            '${count == 1 ? 'session' : '$count sessions'} open on this computer. Their conversations stay in the list to open '
            'again. Or leave it: it is saved, and applies the next time the host starts.'),
        actions: [
          TextButton(key: const Key('apply-profile-later'), onPressed: () => Navigator.pop(context, false), child: const Text('Leave it for now')),
          FilledButton(key: const Key('apply-profile-now'), onPressed: () => Navigator.pop(context, true), child: const Text('Apply now')),
        ],
      ),
    );
    return ok ?? false;
  }

  /// The profiles as canoa.cfg has them now, for every agent the app offers or the file gives a profile.
  Future<void> _showProfiles() async {
    final above = _navigator.currentContext;
    if (above == null) return;
    await showSelectableDialog<void>(
      context: above,
      builder: (_) => ProfilesDialog(
        config: config,
        agents: _profileAgents,
        onLoadout: _showLoadout,
        home: platformProfileHome(configPath: _configPath),
        onPin: (name, digest) async {
          final changed = config.withOverlayPin(name, digest);
          await _write(changed);
          if (mounted) setState(() => config = changed);
          return changed;
        },
      ),
    );
  }

  /// What the conversation shown offers the keyboard and the palette.
  final _conversation = _ConversationHandle();

  /// The conversations shown in a window of their own.
  late final PopoutHub _popout = PopoutHub(store, widget.popoutWindows ?? popout_platform.createPopoutWindows());

  /// This state sits above the app's Navigator, so its dialogs open in the Navigator's context.
  final _navigator = GlobalKey<NavigatorState>();
  final _messenger = GlobalKey<ScaffoldMessengerState>();
  Timer? _contextTimer;
  StreamSubscription<(ChatSession, AttentionKind)>? _attention;
  final List<VerifiedAgent> _ownedClients = [];

  /// Tells the person about [session] outside the window, when its kind is on and they are not looking at it there.
  Future<void> _notify(ChatSession session, AttentionKind kind) async {
    final notifier = widget.notifier;
    final wanted = switch (kind) {
      AttentionKind.permission => config.notifications.permission,
      AttentionKind.question => config.notifications.question,
      AttentionKind.finished => config.notifications.finished,
    };
    if (notifier == null || !wanted) return;
    if (session.id == store.selectedId && await notifier.attended()) return;
    final text = attentionText(session, kind);
    await notifier.show(title: text.title, body: text.body, sessionId: session.id);
  }

  /// The person chose a notification: its conversation, in the window.
  void _openFromNotification(String sessionId) {
    if (!mounted) return;
    store.select(sessionId);
    unawaited(widget.shell?.open());
  }

  void _showWaiting() => unawaited(widget.shell?.showWaiting(store.waiting));
  bool _claimingContext = false;

  /// The tabs the extension's panel shares with a session; null outside the extension.
  TabsComposerExtension? _tabs;

  /// The Documents folder of the machine the agents of a webpage's sessions run on, which the panel cannot read and the person gives.
  final _siteDocuments = ValueNotifier<String?>(null);
  void _keepSiteDocuments() => saveSiteDocuments(_siteDocuments.value);

  Future<AcpPage?> get _provider => widget.browserProvider ?? findAcp(timeout: const Duration(milliseconds: 300));

  Future<void> _claimContext() async {
    if (_claimingContext) return;
    _claimingContext = true;
    try {
      final context = await claimExtensionPageContext();
      if (context != null && mounted) store.addPageContext(context);
    } finally {
      _claimingContext = false;
    }
  }

  /// Each agent the person grants becomes a session. The provider decides the working directory it was granted.
  Future<void> _useBrowserAgents(AcpPage acp, {required String cwd, required String title}) async {
    final agents = await acp.request(purpose: 'Chat with it in Canoa');
    for (final agent in agents) {
      await _openProviderSession(acp, agent, cwd: cwd, title: title);
    }
  }

  Future<void> _openProviderSession(
    AcpPage acp, AcpPageAgent agent, {required String cwd, required String title, String? site, String? workspaceId}) async {
    final client = await connectProviderAgent(acp, agent, decider: store.askPermission, elicitor: store.askQuestion);
    try {
      final opened = await client.openSession(cwd: cwd);
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addProviderSession(
        title: title.isEmpty ? agent.label : title,
        agent: agent.label,
        peerId: client.peerId,
        remoteSessionId: opened.id,
        cwd: cwd,
        site: site,
        workspaceId: workspaceId,
      );
      local.agentNames = title.isEmpty;
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
      await store.applyHostProfile(local, expected: _hasProfiles);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// A new session for [site], from the page in front: each agent the person grants works in a folder of its own, under the
  /// site's folder in the Documents of its machine, and the ledger notes the site.
  Future<void> _newSiteSession(String site) async {
    final documents = _siteDocuments.value;
    if (documents == null) throw StateError('Say where the Documents folder is first');
    final acp = await _provider;
    if (acp == null) throw StateError('This browser has no provider of agents');
    final agents = await acp.request(purpose: 'Chat with it in Canoa about $site');
    for (final agent in agents) {
      final plan = siteScratch(site, template: config.sessions.workspace, documents: documents);
      await _openProviderSession(acp, agent, cwd: plan.folder, title: '', site: site, workspaceId: plan.id);
    }
  }

  /// Opens a session of the site again, in its own folder, through the browser's provider: the agent it ran on must still be
  /// granted to this browser, or the person is asked again.
  Future<void> _resumeSiteSession(LedgerEntry entry) async {
    if (entry.place != SessionPlace.browser) return _reopen(entry);
    final acp = await _provider;
    if (acp == null) throw StateError('This browser has no provider of agents');
    var agent = (await acp.agents()).where((each) => each.peer == entry.peerId).firstOrNull;
    agent ??= (await acp.request(purpose: 'Resume ${entry.title ?? 'a conversation'}')).where((each) => each.peer == entry.peerId).firstOrNull;
    if (agent == null) throw StateError('${entry.entry} is not granted to this browser now');
    final client = await connectProviderAgent(acp, agent, decider: store.askPermission, elicitor: store.askQuestion);
    try {
      await store.reopen(entry, client, expected: _hasProfiles);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// Whether canoa.cfg sets any profile, so an agent given none of it says so.
  bool get _hasProfiles => config.overlays.isNotEmpty || !config.profile.isEmpty || config.agentProfiles.isNotEmpty;

  late final HostControl? _control = widget.host == null ? null : HostControl(widget.host!);

  Worktrees? _localWorktrees() => _control == null ? null : controlWorktrees(_control);

  /// Makes, reads and removes worktrees where a session's agent runs: by this app's host for one it runs, through the host for
  /// one it reached at [endpoint]; null where neither can.
  Worktrees? _worktrees(SessionPlace place, {String? endpoint, String? fallback, String? peerId, String? method}) {
    if (place == SessionPlace.local) return _localWorktrees();
    if (place != SessionPlace.host && place != SessionPlace.peer) return null;
    final uri = Uri.tryParse(endpoint ?? '');
    final proof = PeerMethod.values.where((each) => each.id == method).firstOrNull;
    if (uri == null || proof == null || peerId == null) return null;
    if (isPipeUrl(uri.toString())) {
      return HostWorktrees((name, params) => hostWorktreeRequestOverPipe(
        uri.toString(), expectedPeerId: peerId, method: name, params: params,
        verifyPeer: (channel, expected) => verifyBrowserPeer(channel, expected, proof),
        timeout: Duration(seconds: config.connectTimeout),
      ));
    }
    final overWebTransport = uri.scheme == 'https';
    final endpoints = chatEndpoints(
      webSocket: overWebTransport ? (fallback ?? '') : uri.toString(),
      webTransport: overWebTransport ? uri.toString() : '',
      preferWebTransport: config.preferWebTransport,
    );
    return HostWorktrees((name, params) => hostWorktreeRequest(
      endpoints, expectedPeerId: peerId, method: name, params: params,
      verifyPeer: (channel, expected) => verifyBrowserPeer(channel, expected, proof),
      timeout: Duration(seconds: config.connectTimeout),
    ));
  }

  /// Takes back a worktree made for a session that did not start, which nothing has changed yet.
  static Future<void> _unmake(Worktrees worktrees, SessionWorktree worktree) async {
    try {
      await worktrees.remove(worktree);
    } on Object catch (error) {
      debugPrint('Canoa: the worktree made for a session that did not start stays at ${worktree.path}: $error');
    }
  }

  /// Ends [session], and when it worked in a worktree of its own asks, once its agent has ended, whether to keep it.
  Future<void> _closeSession(ChatSession session) async {
    final worktree = session.worktree;
    final worktrees = worktree == null
        ? null
        : _worktrees(session.place, endpoint: session.endpoint?.toString(), fallback: session.websocketFallback?.toString(),
            peerId: session.peerId, method: session.method);
    await store.close(session);
    if (worktree == null || worktrees == null) return;
    WorktreeChoice? choice;
    WorktreeState? shown;
    do {
      WorktreeState? state;
      String? problem;
      try {
        state = await worktrees.status(worktree);
      } on Object catch (error) {
        problem = '$error';
      }
      if (state != null && !state.exists) {
        _messenger.currentState?.showSnackBar(SnackBar(content: Text('Its worktree ${worktree.path} was already gone.')));
        return;
      }
      // Changes made after the person was told of them are told again before anything is deleted.
      if (choice == WorktreeChoice.discard && state != null && shown != null && state.listing == shown.listing) break;
      final above = _navigator.currentContext;
      if (above == null || !above.mounted) return;
      choice = await showSelectableDialog<WorktreeChoice>(
        context: above,
        builder: (_) => WorktreeCloseDialog(worktree: worktree, state: state, problem: problem),
      );
      shown = state;
    } while (choice == WorktreeChoice.discard);
    if (choice == null || choice == WorktreeChoice.keep) return;
    try {
      final removed = await worktrees.remove(worktree, discard: choice == WorktreeChoice.discard);
      _messenger.currentState?.showSnackBar(SnackBar(content: Text(removed.branchKept
          ? 'Removed the worktree. Branch ${removed.branch} stays, with ${removed.commits} commit(s) of its own.'
          : 'Removed the worktree and its branch ${removed.branch}.')));
    } on Object catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The worktree stays at ${worktree.path}: $error')));
    }
  }

  /// The worktree [entry] worked in, found again, or an error saying it is gone.
  Future<void> _findWorktree(LedgerEntry entry) async {
    final worktree = entry.worktree;
    if (worktree == null) return;
    final worktrees = _worktrees(entry.place, endpoint: entry.endpoint, fallback: entry.fallback, peerId: entry.peerId,
        method: entry.method);
    if (worktrees == null) throw StateError('This session worked in a worktree, which this build cannot reach');
    final state = await worktrees.status(worktree);
    if (!state.exists) {
      throw StateError('The worktree this session worked in is gone: ${worktree.path}, branch ${worktree.branch}');
    }
  }

  /// The agent entries the canoa this app runs is serving now, as it lists them.
  /// The other sources of this computer a session that ran on one can be moved to, by name and as shown.
  Map<String, String> _moveTargets(ChatSession session) => store.canMove(session) && _control != null
      ? {
          for (final source in _hostedSources)
            if (source.local && source.label != session.agent) source.label: source.displayName,
        }
      : const {};

  /// Asks which source to move [session] to, has the host carry its history over, and opens the result beside it.
  Future<void> _moveTo(BuildContext context, ChatSession session) async {
    final control = _control;
    final to = control == null ? null : await pickTarget(context, targets: _moveTargets(session));
    if (to == null || control == null || !mounted || !context.mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    try {
      await moveSession(
        store: store,
        control: await control.ready(),
        session: session,
        to: to,
        connect: (entry) => _linkAgent(entry, _cacheMeta((lifetime: CacheLifetime.chat, chosen: false))),
        keep: _ownedClients.add,
      );
    } on Object catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That session could not be moved: ${reasonOf(error)}')));
    }
  }

  List<AgentSource> get _hostedSources => hostedSources(widget.host?.listening ?? const []);

  /// The sources Settings has: the host's own that it does not hide, canoa.cfg's `[agents.NAME]` commands, and the other agent
  /// entries of the canoa this app runs.
  List<AgentSource> get _sources => [
    for (final source in _hostedSources)
      if (source.local && !config.hiddenSources.contains(source.label)) source,
    for (final source in _hostedSources)
      if (!source.local) source,
  ];

  /// What each source needs and whether it is here, as last checked.
  Map<AgentSource, SourceCheck> _checks = const {};

  Future<SourceCheck> _check(AgentSource source) async {
    final listening = widget.host?.listening.where((listening) => listening.name == source.hostEntry).firstOrNull;
    if (listening == null) {
      return SourceCheck(problem: 'canoa is not serving ${source.label} now: open Hosting to see why.');
    }
    // The Store's extension for the source's program says whether it is here, which version and whether a newer one exists;
    // only a source no extension serves keeps the host's own list of what it is missing.
    if (_programs case final programs?) {
      final reported = await storeReadiness(programs, source);
      if (reported != null) return reported;
    }
    return SourceCheck(missing: listening.missing);
  }

  /// The programs the host's Store keeps. Nothing is asked of the host until a page that shows them, or a source's readiness,
  /// needs it; an install runs only when the person confirms its plan.
  late final ProgramStores? _programs = _control == null
      ? null
      : (ProgramStores(control: () => _control.ready(wait: false), open: _control.open)..addListener(_programsChanged));

  String? _programsSeen;

  /// A program found, installed, updated or removed changes whether its sources can run.
  void _programsChanged() {
    final now = _programs?.signature;
    if (now == _programsSeen) return;
    _programsSeen = now;
    if (mounted && _checked) unawaited(_checkSources());
  }

  /// What `canoa options` says of [source]: the models and efforts its provider lists with no session. A preset the host runs
  /// is asked by its harness, a hosted entry by its name in canoa.cfg; any other source lists them only once it has started.
  Future<SourceChoices?> _askChoices(AgentSource source) async {
    if (widget.askChoices case final ask?) return ask(source);
    final tool = widget.host?.tool;
    if (tool == null) return null;
    final List<String> args;
    if (source.local) {
      // Only a harness lists its models before it starts.
      final harness = source.harness;
      if (harness == null) return null;
      args = ['options', harness, '--format', 'json'];
    } else if (source.hostEntry != null && _configPath != null) {
      args = ['options', source.hostEntry!, '--entry', '--config', _configPath!, '--format', 'json'];
    } else {
      return null;
    }
    final ran = await tool.run(args);
    return ran.exitCode == 0 ? SourceChoices.parse(ran.stdout) : null;
  }

  /// Whether the sources have been looked for: only once a new session or Settings needs them, since each runs a lookup.
  bool _checked = false;

  Future<void> _checkSources() async {
    _checked = true;
    // A change of the Store's programs can come while a page is built, and a source-less host has nothing to wait for.
    await null;
    final sources = _sources;
    final checks = {for (final source in sources) source: await _check(source)};
    if (mounted) setState(() => _checks = checks);
  }

  /// The source and folder chosen last, which a new session starts with.
  AgentSource? _lastSource;
  String? _lastFolder;

  void _noteLast() {
    final session = store.selected;
    if ((session == null || session.isDraft) && !_checked) unawaited(_checkSources());
    if (session == null || session.source == null) return;
    _lastSource = session.source;
    if (session.isDraft && session.cwd != null) _lastFolder = session.cwd;
    if (session.isDraft) _hint(session);
  }

  /// Folders sessions worked in, the last chosen first: a worktree's stands for its repository. One that is gone is left out.
  List<String> get _recentFolders => {
    ?_lastFolder,
    for (final session in store.sessions)
      if (session.source != null && session.cwd != null) session.worktree?.repo ?? session.cwd!,
    for (final entry in store.ledger?.entries ?? const <LedgerEntry>[])
      if (entry.place == SessionPlace.local || entry.place == SessionPlace.host)
        ?(entry.worktree?.repo ?? entry.cwd),
  }.where(widget.folderExists ?? folderExists).take(8).toList();

  /// A new session with nothing started, from the source and folder chosen last; one already waiting untouched is shown
  /// instead of another.
  void _newSession() {
    if (!canRunLocalAgents && widget.host == null) {
      store.createPreviewSession();
      return;
    }
    for (final session in store.sessions) {
      if (session.status == SessionStatus.draft && session.messages.isEmpty && session.draftText.trim().isEmpty &&
          session.attachments.isEmpty) {
        store.select(session.id);
        return;
      }
    }
    if (!_checked) unawaited(_checkSources());
    final sources = _sources;
    final source = sources.contains(_lastSource)
        ? _lastSource
        : sources.where((source) => _checks[source]?.ready ?? false).firstOrNull ?? sources.firstOrNull;
    final draft = store.createDraft(source: source);
    _hint(draft);
  }

  /// The draft and source each draft's profile defaults were last worked out for.
  final Map<String, AgentSource?> _hinted = {};

  /// Puts the profile's model, effort and mode as [draft]'s defaults, where it has them.
  void _hint(ChatSession draft) {
    final source = draft.source;
    if (_hinted.containsKey(draft.id) && _hinted[draft.id] == source) return;
    _hinted[draft.id] = source;
    if (source == null) return;
    // A hosted agent is given none of the profile, so its defaults are only the person's last choices.
    store.defaultChoices(draft);
  }

  /// Starts [draft]'s agent on its first message: the worktree is made, the profile prepared and shown where it has something
  /// to approve, the agent started and its session opened, and the draft bound to it. Anything made is taken back when it
  /// fails, and the draft stays as it was.
  Future<void> _startDraft(ChatSession draft) async {
    final source = draft.source;
    if (source == null) throw StateError('Choose where its agent comes from first');
    final check = _checks[source] ?? await _check(source);
    if (!check.ready) {
      throw StateError(check.describe(source.label)!);
    }
    // With none chosen the session works in its scratch folder, which is made now and not before.
    final scratch = draft.cwd == null || draft.cwd!.isEmpty;
    final folder = scratch ? _scratchOf(draft) : draft.cwd;
    if (folder == null) throw StateError('Choose its folder first');
    var madeFolder = false;
    if (scratch) {
      try {
        madeFolder = await makeFolder(folder);
      } on Object catch (error) {
        throw StateError('Cannot make its folder $folder: ${reasonOf(error)}');
      }
    }
    // Worktree shows, and applies, only for a folder in a git repository.
    final worktree = !scratch && draft.wantsWorktree && (widget.isGitRepository ?? inGitRepository)(folder);
    final starting = _Starting();
    _inFlight.add(starting);
    try {
      await _startHosted(draft, source.hostEntry!, folder, worktree, starting);
    } catch (_) {
      if (madeFolder) await removeEmptyFolder(folder);
      rethrow;
    } finally {
      _inFlight.remove(starting);
    }
  }

  /// Starts under way, whose worktree and agent quitting takes back.
  final Set<_Starting> _inFlight = {};
  bool _quitting = false;

  /// Quitting during a start: what it made so far goes, and it goes no further.
  Future<void> _endStarts() async {
    _quitting = true;
    for (final starting in [..._inFlight]) {
      await starting.client?.close();
      if (starting.made case final made?) await _unmake(starting.worktrees!, made);
    }
  }

  /// [chosen] in canoa.cfg's order, without any that is no longer installed.
  List<String> _overlaysNamed(Set<String> chosen) => [for (final name in config.overlays.keys) if (chosen.contains(name)) name];

  /// What `initialize` carries to a host to say how long this session's prompt cache should live.
  Map<String, Object?> _cacheMeta(({CacheLifetime lifetime, bool chosen}) asked) =>
      {cacheLifetimeMeta: cacheLifetimeRequest(asked.lifetime, chosen: asked.chosen)};

  Future<void> _startHosted(ChatSession draft, String name, String folder, bool worktree, _Starting starting) async {
    final host = widget.host;
    final listening = host?.listening.where((listening) => listening.name == name).firstOrNull;
    if (host == null || listening == null) throw StateError('canoa is not serving $name now: open Hosting to see why');
    if (listening.local) return _startViaLink(draft, name, folder, worktree, starting);
    final method = PeerMethod.values.where((method) => method.id == (listening.method ?? host.peerMethod)).firstOrNull;
    if (method == null) throw StateError('canoa did not say which method it proves by');
    final endpoint = Uri.parse(listening.url);
    final worktrees = worktree
        ? _worktrees(SessionPlace.host, endpoint: listening.url, peerId: listening.peer, method: method.id)
        : null;
    if (worktree && worktrees == null) throw StateError('Canoa cannot ask canoa for a worktree');
    final made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(name));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    try {
      client = isPipeUrl(listening.url)
          ? await connectBrowserAcpOverPipe(
              listening.url, pinnedPeerId: listening.peer, method: method,
              timeout: Duration(seconds: config.connectTimeout),
              decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
              initializeMeta: _cacheMeta(store.cacheLifetimeOf(draft)),
            )
          : await connectBrowserAcp(
              chatEndpoints(webSocket: listening.url, webTransport: '', preferWebTransport: config.preferWebTransport),
              pinnedPeerId: listening.peer, method: method,
              timeout: Duration(seconds: config.connectTimeout),
              decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
              initializeMeta: _cacheMeta(store.cacheLifetimeOf(draft)),
            );
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made, place: SessionPlace.host, endpoint: endpoint,
          method: method.id);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    await store.applyHostProfile(draft, expected: _hasProfiles);
  }

  /// Answers the host's question, before a session of [agent] starts, of which MCP servers and which overlays' hooks it may
  /// start: the person's choice for this session, kept in canoa.cfg when they ask. Nothing is allowed when there is no window.
  ProfileApprover _approver(String agent) => (ask) async {
    final above = _navigator.currentContext;
    if (above == null || (ask.servers.isEmpty && ask.hooks.isEmpty)) return noApproval;
    final choice = await showSelectableDialog<ApprovalChoice>(
      context: above,
      builder: (_) => ProfileApprovalDialog(ask: ask, agent: agent),
    );
    if (choice == null) return noApproval;
    if (choice.remember && (choice.allow.isNotEmpty || choice.hooks.isNotEmpty) && mounted) {
      final kept = <String, Object>{
        ...?config.agentMcp[agent],
        for (final server in ask.servers)
          if (choice.allow.contains(server.name)) server.name: server.digest,
      };
      final keptHooks = <String, String>{
        ...?config.agentHooks[agent],
        for (final hook in ask.hooks)
          if (choice.hooks.contains(hook.overlay)) hook.overlay: hook.digest,
      };
      try {
        final changed = config.withAgentMcp(agent, kept).withAgentHooks(agent, keptHooks);
        await _write(changed);
        if (mounted) setState(() => config = changed);
      } on Object catch (error) {
        debugPrint('Canoa could not keep the choices for $agent: $error');
      }
    }
    return (servers: choice.allow, hooks: choice.hooks);
  };

  /// The agent behind the host's own source [name], reached through the in-process link and ready to open a session.
  Future<VerifiedAgent> _linkAgent(String name, Map<String, Object?> initializeMeta) async {
    final host = widget.host;
    if (host == null) throw StateError('This build has no host of its own to start $name');
    final HostLink link;
    try {
      link = host.openLink(name);
    } on HostCommandFailed catch (error) {
      throw StateError('$error');
    }
    final client = VerifiedAgent.fromChild(
      label: name, messages: link.messages, send: link.send, close: link.close,
      decider: store.askPermission, elicitor: store.askQuestion, initializeMeta: initializeMeta,
      approver: _approver(name),
    );
    try {
      await client.initialize();
      return client;
    } on Object {
      await client.close();
      rethrow;
    }
  }

  /// Starts a session on one of the host's own sources, over the in-process link: no address and no proof, since the host
  /// is in this process.
  Future<void> _startViaLink(ChatSession draft, String name, String folder, bool worktree, _Starting starting) async {
    final worktrees = worktree ? (widget.worktrees ?? _localWorktrees)() : null;
    if (worktree && worktrees == null) throw StateError('This build cannot make a worktree');
    final made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(name));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    try {
      // The overlays it starts with are Loadout's defaults unless the person chose; what it starts with is kept with it.
      final chosen = draft.overlays?.where(config.overlays.containsKey).toSet() ?? config.defaultOverlays.toSet();
      if (config.overlays.isNotEmpty) draft.overlays = chosen;
      client = await _linkAgent(name, {
        ..._cacheMeta(store.cacheLifetimeOf(draft)),
        if (config.overlays.isNotEmpty) overlaysMeta: _overlaysNamed(chosen),
      });
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    await store.applyHostProfile(draft, expected: _hasProfiles);
  }

  /// Opens a session from the list again: its agent is started or reached again, then asked to open it.
  Future<void> _reopen(LedgerEntry entry) async {
    for (final session in store.sessions) {
      if (session.peerId == entry.peerId && session.remoteSessionId == entry.sessionId) {
        store.select(session.id);
        return;
      }
    }
    final why = entry.whyNotReopened;
    if (why != null) throw StateError(why);
    await _findWorktree(entry);
    final VerifiedAgent client;
    switch (entry.place) {
      case SessionPlace.local when _hostedSources.any((source) => source.local && source.label == entry.entry):
        client = await _linkAgent(entry.entry, {
          ..._cacheMeta((
            lifetime: CacheLifetime.named(entry.cacheLifetime) ?? CacheLifetime.chat,
            chosen: entry.cacheLifetime != null,
          )),
          if (entry.overlays case final started?) overlaysMeta: _overlaysNamed(started.toSet()),
        });
      case SessionPlace.local:
        throw StateError('${entry.entry} is not one of the sources of this computer now');
      case SessionPlace.host || SessionPlace.peer:
        final endpoint = Uri.tryParse(entry.endpoint ?? '');
        final method = PeerMethod.values.where((method) => method.id == entry.method).firstOrNull;
        if (endpoint == null || method == null) throw StateError('Canoa did not note how to reach that peer');
        final overWebTransport = endpoint.scheme == 'https';
        final cacheMeta = _cacheMeta((
          lifetime: CacheLifetime.named(entry.cacheLifetime) ?? CacheLifetime.chat,
          chosen: entry.cacheLifetime != null,
        ));
        client = isPipeUrl(endpoint.toString())
            ? await connectBrowserAcpOverPipe(
                endpoint.toString(), pinnedPeerId: entry.peerId, method: method,
                timeout: Duration(seconds: config.connectTimeout),
                decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
                initializeMeta: cacheMeta,
              )
            : await connectBrowserAcp(
                chatEndpoints(
                  webSocket: overWebTransport ? (entry.fallback ?? '') : endpoint.toString(),
                  webTransport: overWebTransport ? endpoint.toString() : '',
                  preferWebTransport: config.preferWebTransport,
                ),
                pinnedPeerId: entry.peerId, method: method,
                timeout: Duration(seconds: config.connectTimeout),
                decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
                initializeMeta: cacheMeta,
              );
      case SessionPlace.browser:
        throw StateError(why ?? 'Reopen it from Earlier conversations');
    }
    try {
      await store.reopen(entry, client, expected: _hasProfiles);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// The host's entries come and go as it starts and ends, and a hosted source can run only while it serves one.
  List<AgentSource>? _serving;
  void _hostChanged() {
    final serving = _hostedSources;
    if (_serving != null && _serving!.length == serving.length && _serving!.every(serving.contains)) return;
    _serving = serving;
    store.resolveKeptSources((label) =>
        _sources.where((source) => source.label == label).firstOrNull);
    if (_checked) {
      unawaited(_checkSources());
    } else if (mounted) {
      setState(() {});
    }
  }

  /// Ends every agent this app started, and the host with the agents it started.
  Future<void> _endAgents() async {
    await _endStarts();
    await store.ledger?.flush();
    await store.drafts?.flush();
    final ending = [for (final client in _ownedClients) client.close(), ?widget.host?.stop()];
    _ownedClients.clear();
    // Longer than the host has to end its agents.
    await Future.wait(ending).timeout(const Duration(seconds: 10), onTimeout: () => const []);
  }

  Future<void> _connect(ConnectTarget target) async {
    final endpoints = chatEndpoints(
      webSocket: target.webSocket,
      webTransport: target.webTransport,
      preferWebTransport: config.preferWebTransport,
    );
    final ws = endpoints.webSocket!;
    final wt = target.webTransport.isEmpty ? null : Uri.parse(target.webTransport);
    final pinnedPeerId = target.peerId.toLowerCase();
    final place = widget.host?.peerId?.toLowerCase() == pinnedPeerId ? SessionPlace.host : SessionPlace.peer;
    // The host makes it inside its working roots, before any agent starts, and says where.
    final worktrees = target.worktree
        ? _worktrees(place, endpoint: (wt ?? ws).toString(), fallback: wt == null ? null : ws.toString(),
            peerId: pinnedPeerId, method: target.method.id)
        : null;
    if (target.worktree && worktrees == null) throw StateError('Canoa cannot ask this agent\'s host for a worktree');
    final made = worktrees == null
        ? null
        : await worktrees.create(target.cwd, worktreeBranch(target.title.isEmpty ? 'remote agent' : target.title));
    final cwd = made?.path ?? target.cwd;
    final VerifiedAgent client;
    try {
      client = await connectBrowserAcp(
        endpoints, pinnedPeerId: pinnedPeerId, method: target.method,
        timeout: Duration(seconds: config.connectTimeout),
        decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
      );
    } catch (_) {
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    try {
      final opened = await client.openSession(cwd: cwd);
      final remoteId = opened.id;
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addRemoteSession(
        title: target.title.isEmpty ? 'Remote agent' : target.title,
        agent: pinnedPeerId.substring(0, 22),
        peerId: pinnedPeerId,
        remoteSessionId: remoteId,
        endpoint: wt ?? ws,
        websocketFallback: wt == null ? null : ws,
        place: place,
        method: target.method.id,
        cwd: cwd,
        worktree: made,
      );
      local.agentNames = target.title.isEmpty;
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
      await store.applyHostProfile(local, expected: _hasProfiles);
    } catch (_) {
      await client.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
  }

  @override
  void initState() {
    super.initState();
    ownsStore = widget.store == null;
    config = widget.config;
    widget.settingsSignal?.addListener(_requestSettings);
    widget.hostingSignal?.addListener(_requestHosting);
    store = widget.store ??
        SessionStore(recorder: updateRecorderFromEnvironment(), ledger: widget.ledger, rules: widget.rules, defaultMode: config.defaultMode, bypassOnceOnly: config.allowOnceOnly,
            drafts: widget.drafts, embedLimit: config.embedLimit);
    store.starter = _startDraft;
    store.choicesLoader = _askChoices;
    store.addListener(_noteLast);
    widget.host?.addListener(_hostChanged);
    unawaited(store.restoreDrafts((label) =>
        _sources.where((source) => source.label == label).firstOrNull));
    // Closing a window to the tray keeps its agents. Quitting does not, and `dispose` does not run when the window is destroyed.
    widget.shell?.beforeQuit = _endAgents;
    _conversation.close = _closeSession;
    _conversation
      ..moveTargets = _moveTargets
      ..move = _moveTo;
    _conversation.popout = _popout;
    _attention = store.attention.listen((needs) => unawaited(_notify(needs.$1, needs.$2)));
    unawaited(widget.notifier?.initialize(_openFromNotification));
    store.addListener(_showWaiting);
    // The welcome page, which a window with no session shows, lists each source with what it needs.
    scheduleMicrotask(_noteLast);
    unawaited(_claimContext());
    final bridge = widget.tabBridge;
    _tabs = bridge == null ? TabsComposerExtension.platform(store) : TabsComposerExtension(TabShares(bridge), store: store);
    if (_tabs != null) {
      _siteDocuments.value = loadSiteDocuments() ?? widget.documents;
      _siteDocuments.addListener(_keepSiteDocuments);
    }
    _contextTimer = Timer.periodic(
      const Duration(seconds: 2), (_) => unawaited(_claimContext()),
    );
  }

  @override
  void didUpdateWidget(CanoaApp oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.config != oldWidget.config) config = widget.config;
  }

  @override
  void dispose() {
    _popout.dispose();
    _siteDocuments.removeListener(_keepSiteDocuments);
    _siteDocuments.dispose();
    _tabs?.dispose();
    _contextTimer?.cancel();
    unawaited(_attention?.cancel());
    store.removeListener(_showWaiting);
    store.removeListener(_noteLast);
    if (identical(store.starter, _startDraft)) store.starter = null;
    if (identical(store.choicesLoader, _askChoices)) store.choicesLoader = null;
    widget.host?.removeListener(_hostChanged);
    unawaited(_control?.close());
    widget.settingsSignal?.removeListener(_requestSettings);
    widget.hostingSignal?.removeListener(_requestHosting);
    widget.shell?.beforeQuit = null;
    if (ownsStore) store.dispose();
    for (final client in _ownedClients) {
      unawaited(client.close());
    }
    super.dispose();
  }

  /// The folder [session] works in when none is chosen, from canoa.cfg's `[sessions] workspace`; null for a session not made here.
  String? _scratchOf(ChatSession session) {
    final id = session.workspaceId;
    return id == null ? null : scratchFolder(config.sessions.workspace, id, source: session.source, now: session.created, documents: widget.documents);
  }

  /// What the composer offers, from this app's sources, folders, files and clipboard.
  ComposerKit get _kit => ComposerKit(
    sources: _sources,
    checks: _checks,
    recentFolders: _recentFolders,
    inGitRepository: widget.isGitRepository ?? inGitRepository,
    pickFolder: widget.pickFolder ?? (canRunLocalAgents ? () => getDirectoryPath() : null),
    scratchFolder: hasWorkspaceFolders && canRunLocalAgents ? _scratchOf : null,
    pickFiles: widget.pickFiles ?? () => openFiles(),
    statFile: widget.statFile ?? (canRunLocalAgents ? localFileSize : null),
    clipboard: widget.clipboard ?? const SystemClipboard(),
    overlays: config.overlays.keys.toList(),
    defaultOverlays: config.defaultOverlays,
    onLoadout: _showLoadout,
    pasteAsFile: config.pasteAsFile,
    warnPermille: config.warnPermille,
    onSourceSettings: () => unawaited(_showSettings(sources: true)),
    extension: _tabs,
    onProfile: (session) {
      final above = _navigator.currentContext;
      final report = session.profile;
      if (above == null || report == null) return;
      unawaited(showSelectableDialog<void>(context: above, builder: (_) => ProfileReportDialog(report: report)));
    },
  );

  /// In the extension's panel, the sessions of the page in front above the welcome page.
  Widget _withSite(Widget welcome) {
    final tabs = _tabs?.tabs, ledger = store.ledger;
    if (tabs == null || ledger == null) return welcome;
    return Column(children: [
      // At most half the panel: a site with many sessions scrolls inside it.
      Flexible(
        child: SingleChildScrollView(
          child: SiteSessions(
            tabs: tabs, ledger: ledger, store: store, documents: _siteDocuments, template: config.sessions.workspace,
            onNew: _newSiteSession, onResume: _resumeSiteSession,
          ),
        ),
      ),
      Expanded(child: welcome),
    ]);
  }

  @override
  Widget build(BuildContext context) {
    _popout
      ..look = (_preview ?? config).look.name
      ..density = (_preview ?? config).density.name;
    return MaterialApp(
      navigatorKey: _navigator,
      scaffoldMessengerKey: _messenger,
      title: 'Canoa',
      debugShowCheckedModeBanner: false,
      theme: themeFor((_preview ?? config).look, Brightness.light, density: (_preview ?? config).density),
      darkTheme: themeFor((_preview ?? config).look, Brightness.dark, density: (_preview ?? config).density),
      themeMode: (_preview ?? config).themeMode,
      home: AnimatedBuilder(
        animation: Listenable.merge([store, _popout]),
        builder: (context, _) => _CanoaHome(
          store: store, onConnect: _connect, onUseBrowserAgents: _useBrowserAgents, onNewSession: _newSession, onSettings: _showSettings,
          onHosting: widget.host == null ? null : _showHosting, onLoadout: _showLoadout, onReopen: _reopen,
          conversation: _conversation,
          kit: _kit,
          renderers: widget.renderers,
          welcome: _withSite(WelcomeView(
            sources: _sources, checks: _checks, programs: _programs, onNewSession: _newSession,
            onSettings: () => unawaited(_showSettings(sources: true)),
          )),
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onNewSession, required this.onSettings,
    this.onHosting, this.onLoadout, this.onReopen, required this.conversation, this.kit = const ComposerKit(),
    this.renderers = chat.ChatRenderers.none, required this.welcome,
  });

  final SessionStore store;
  final Widget welcome;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;

  /// Opens a new session's empty view.
  final VoidCallback onNewSession;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;
  final VoidCallback? onLoadout;
  final Future<void> Function(LedgerEntry)? onReopen;
  final _ConversationHandle conversation;

  void _connect(BuildContext context) => unawaited(showCanoaConnectDialog(
    context,
    onConnect,
    browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
    onUseBrowserAgents: onUseBrowserAgents,
  ));

  /// The session [step] places from the one chosen, going round the list.
  void _step(int step) {
    final sessions = listOrder(store.sessions);
    if (sessions.isEmpty) return;
    final at = sessions.indexWhere((session) => session.id == store.selectedId);
    store.select(sessions[(at + step) % sessions.length].id);
  }

  /// The agent's first option of [kind] for the question shown, if any.
  void _answer(String kind) {
    final session = store.selected;
    if (session == null || session.pendingPermissions.isEmpty) return;
    final options = session.pendingPermissions.first.options;
    final chosen = options.where((option) => option.kind == kind).firstOrNull ??
        (kind == 'reject_once' ? options.where((option) => option.rejects).firstOrNull : null);
    if (chosen != null) store.choosePermission(session, chosen);
  }

  void _stop() {
    if (store.selected?.status == SessionStatus.working) unawaited(store.cancelActiveSession().catchError((Object _) {}));
  }

  /// The rail as the window shows it: beside the conversation, or in the drawer.
  Widget _rail(BuildContext context) => SessionRail(
    store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onNewSession: onNewSession,
    onSettings: onSettings, onHosting: onHosting, onLoadout: onLoadout, onReopen: onReopen,
    onSearch: () => unawaited(_palette(context, sessionsOnly: true)),
    onClose: conversation.close ?? store.close,
  );

  Future<void> _palette(BuildContext context, {bool sessionsOnly = false}) {
    final session = store.selected;
    return showSelectableDialog<void>(
      context: context,
      builder: (_) => CommandPalette(items: sessionsOnly ? [
        for (final other in listOrder(store.sessions))
          PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
      ] : [
        PaletteItem('New session', onNewSession, icon: Icons.add_comment_outlined),
        PaletteItem('Connect to an agent', () => _connect(context), icon: Icons.link_rounded),
        if (session != null && conversation.showChanges != null && sessionChanges(session).isNotEmpty)
          PaletteItem('Show the changes', conversation.showChanges!, icon: Icons.difference_outlined),
        if (session?.status == SessionStatus.working) PaletteItem('Stop the turn', _stop, icon: Icons.stop_circle_outlined),
        PaletteItem('Settings', onSettings, icon: Icons.settings_outlined),
        if (onLoadout != null) PaletteItem('Loadout', onLoadout!, icon: Icons.tune),
        if (onHosting != null) PaletteItem('Hosting', onHosting!, icon: Icons.dns_outlined),
        PaletteItem('Keyboard shortcuts', () => unawaited(showShortcutHelp(context)), icon: Icons.keyboard_outlined),
        for (final other in store.sessions)
          if (other.id != store.selectedId)
            PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
        for (final command in session?.commands ?? const <AgentCommandInfo>[])
          PaletteItem('/${command.name}', () => conversation.insert?.call('/${command.name} '),
              detail: command.description.isEmpty ? 'A command of the agent\'s' : command.description, icon: Icons.terminal),
      ]),
    );
  }

  /// The keyboard's shortcuts, wherever focus is in the window.
  Map<ShortcutActivator, VoidCallback> _shortcuts(BuildContext context) => {
    chord(LogicalKeyboardKey.keyK): () => unawaited(_palette(context)),
    chord(LogicalKeyboardKey.keyN): onNewSession,
    for (final (index, key) in const [
      LogicalKeyboardKey.digit1, LogicalKeyboardKey.digit2, LogicalKeyboardKey.digit3,
      LogicalKeyboardKey.digit4, LogicalKeyboardKey.digit5, LogicalKeyboardKey.digit6,
      LogicalKeyboardKey.digit7, LogicalKeyboardKey.digit8, LogicalKeyboardKey.digit9,
    ].indexed)
      chord(key): () {
        final ordered = listOrder(store.sessions);
        if (index < ordered.length) store.select(ordered[index].id);
      },
    chord(LogicalKeyboardKey.tab): () => _step(1),
    chord(LogicalKeyboardKey.tab, shift: true): () => _step(-1),
    chord(LogicalKeyboardKey.keyL): () => conversation.focusComposer?.call(),
    chord(LogicalKeyboardKey.period): _stop,
    chord(LogicalKeyboardKey.keyA, shift: true): () => _answer('allow_once'),
    chord(LogicalKeyboardKey.keyR, shift: true): () => _answer('reject_once'),
    chord(LogicalKeyboardKey.keyD, shift: true): () => conversation.showChanges?.call(),
    chord(LogicalKeyboardKey.slash): () => unawaited(showShortcutHelp(context)),
  };

  @override
  Widget build(BuildContext context) {
    return CallbackShortcuts(
      bindings: _shortcuts(context),
      child: Focus(autofocus: true, child: _layout(context)),
    );
  }

  Widget _layout(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = constraints.maxWidth < 730 ||
          Uri.base.queryParameters['view'] == 'panel';

      return Selectable(child: Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _rail(context)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: context.look.railWidth,
                  child: _rail(context),
                ),
              Expanded(
                child: _Conversation(
                  key: ValueKey(store.selectedId ?? 'empty'),
                  store: store,
                  narrow: narrow,
                  handle: conversation,
                  kit: kit,
                  renderers: renderers,
                  welcome: welcome,
                ),
              ),
            ],
          ),
        ),
      ));
    });
  }
}

/// What a start under way has made so far, for quitting to take back.
final class _Starting {
  Worktrees? worktrees;
  SessionWorktree? made;
  VerifiedAgent? client;
}

/// What the conversation shown offers the keyboard and the palette, set by it while it is shown.
final class _ConversationHandle {
  Object? owner;
  VoidCallback? focusComposer, showChanges;
  ValueChanged<String>? insert;

  /// Ends a conversation as the app does, asking after its worktree; set by the app.
  Future<void> Function(ChatSession)? close;

  /// The sources a session can move to, by name and as shown, and the move itself; set by the app.
  Map<String, String> Function(ChatSession)? moveTargets;
  Future<void> Function(BuildContext, ChatSession)? move;
  /// The conversations shown in a window of their own; set by the app.
  PopoutHub? popout;
}

class _Conversation extends StatefulWidget {
  const _Conversation({
    super.key, required this.store, required this.narrow, required this.handle,
    this.kit = const ComposerKit(), this.renderers = chat.ChatRenderers.none, required this.welcome,
  });

  final Widget welcome;
  final _ConversationHandle handle;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;

  final SessionStore store;
  final bool narrow;


  @override
  State<_Conversation> createState() => _ConversationState();
}

class _ConversationState extends State<_Conversation> {
  final TextEditingController _input = TextEditingController();

  /// The composer's focus.
  final FocusNode _composerFocus = FocusNode();

  /// The conversation, which keeps its end in view while the person has not scrolled away.
  final _transcript = GlobalKey<chat.ChatTranscriptState>();

  /// A draft's composer text goes with it, across sessions shown and runs of the app.
  void _keepText() {
    final session = widget.store.selected;
    if (session != null && session.isDraft) widget.store.keepDraftText(session, _input.text);
  }

  @override
  void initState() {
    super.initState();
    final session = widget.store.selected;
    // A draft's text, or a first message not yet taken by its agent.
    if (session != null && (session.isDraft || session.draftText.isNotEmpty)) _input.text = session.draftText;
    _input.addListener(_keepText);
    widget.handle
      ..owner = this
      ..focusComposer = _composerFocus.requestFocus
      ..showChanges = () {
        final session = widget.store.selected;
        if (session != null && sessionChanges(session).isNotEmpty) _showChanges(session);
      }
      ..insert = (text) {
        _input.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
        _composerFocus.requestFocus();
      };
  }

  /// Whether the changes pane is beside the conversation, on a wide window.
  bool _changesOpen = false;

  /// Adds [text] to the message being written, on a line of its own.
  void _addToComposer(String text) {
    final had = _input.text;
    final joined = had.isEmpty || had.endsWith('\n') ? '$had$text' : '$had\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
  }

  /// The changes pane: beside the conversation on a wide window, a sheet over it on a narrow one.
  void _showChanges(ChatSession session) {
    if (!widget.narrow) {
      setState(() => _changesOpen = !_changesOpen);
      return;
    }
    unawaited(showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (sheet) => Selectable(child: FractionallySizedBox(
        heightFactor: 0.9,
        child: ListenableBuilder(
          listenable: widget.store,
          builder: (context, _) => SessionChangesPane(
            store: widget.store,
            session: session,
            onComment: (text) {
              _addToComposer(text);
              Navigator.pop(sheet);
            },
            onClose: () => Navigator.pop(sheet),
          ),
        ),
      )),
    ));
  }

  @override
  void dispose() {
    // A conversation chosen next may have taken the handle already.
    if (identical(widget.handle.owner, this)) {
      widget.handle
        ..owner = null
        ..focusComposer = null
        ..showChanges = null
        ..insert = null;
    }
    _input.removeListener(_keepText);
    _composerFocus.dispose();
    _input.dispose();
    super.dispose();
  }

  Future<void> _showEarlier(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final picked = await showSelectableDialog<AcpSessionInfo>(
      context: context,
      builder: (context) => EarlierConversations(list: (cursor) => widget.store.earlierSessions(session, cursor: cursor)),
    );
    if (picked == null) return;
    try {
      await widget.store.openEarlier(session, picked);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That conversation could not be opened: $error')));
    }
  }

  void _send() {
    final text = _input.text;
    final session = widget.store.selected;
    final attached = session?.attachments.isNotEmpty ?? false;
    if (text.trim().isEmpty && !attached) return;
    // A draft's first message stays in the composer until its agent has started and taken it, so a start that fails
    // leaves it to send again.
    final starting = session != null && session.isDraft;
    try {
      if (!starting) _input.clear();
      unawaited(widget.store.sendText(text).then((_) {
        if (starting && mounted && _input.text == text) _input.clear();
      }).catchError((Object error) {
        if (!mounted) return;
        // Not sent, so it is there to change and send again.
        if (_input.text.isEmpty) _input.text = text;
        final started = session?.remoteSessionId != null;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${starting && !started ? 'Not started' : 'Message not accepted'}: ${reasonOf(error)}')),
        );
      }));
    } on StateError {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
        content: Text('Connect and verify an ACP peer before sending.'),
      ));
    }
    _transcript.currentState?.jumpToLatest();
  }

  /// Under the title: the agent, and once a session made here has started, its folder and worktree, which stay as they are.
  static String _subtitle(ChatSession? session) => session == null
      ? 'Multi-session agent chat'
      : [
          session.agent,
          if (!session.isDraft && session.source != null) ?session.cwd,
          if (session.worktree case final worktree?) 'worktree ${worktree.branch}',
        ].join(' · ');

  /// The state a session's header names.
  static String _state(ChatSession? session) => switch (session?.status) {
    null => 'Offline',
    SessionStatus.preview => 'Preview',
    SessionStatus.draft => 'New',
    SessionStatus.connecting when session!.isDraft => 'Starting',
    final status => status.name,
  };

  Future<void> _popOut(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    try {
      await widget.handle.popout!.popOut(session);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('Could not open a window: ${error is StateError ? error.message : error}')));
    }
  }

  /// What the header offers for [session], beside closing it: as buttons, or under one menu where two or more would crowd a phone.
  List<Widget> _headerActions(ChatSession? session) {
    final actions = <(String, IconData, VoidCallback)>[
      if (session != null && widget.store.canListEarlier(session))
        ('Earlier conversations', Icons.history, () => unawaited(_showEarlier(session))),
      if (session != null && sessionChanges(session).isNotEmpty) ('Changes', Icons.difference_outlined, () => _showChanges(session)),
      if (session != null && (widget.handle.moveTargets?.call(session).isNotEmpty ?? false))
        ('Move to…', Icons.drive_file_move_outlined, () => unawaited(widget.handle.move!(context, session))),
      if (session != null && (!widget.narrow || Uri.base.queryParameters['view'] == 'panel') && !session.isDraft && (widget.handle.popout?.available ?? false))
        if (widget.handle.popout!.isPoppedOut(session))
          ('Bring back from its own window', Icons.close_fullscreen, () => unawaited(widget.handle.popout!.dockBack(session)))
        else
          ('Open in its own window', Icons.open_in_new, () => unawaited(_popOut(session))),
      if (session?.profile case final report?)
        ('Profile', Icons.tune, () => unawaited(showSelectableDialog<void>(
          context: context,
          builder: (_) => ProfileReportDialog(report: report),
        ))),
    ];
    if (!widget.narrow || actions.length < 2) {
      return [
        for (final (tooltip, icon, onPressed) in actions) IconButton(tooltip: tooltip, icon: Icon(icon), onPressed: onPressed),
      ];
    }
    return [
      PopupMenuButton<VoidCallback>(
        tooltip: 'More',
        onSelected: (chosen) => chosen(),
        itemBuilder: (context) => [
          for (final (label, icon, onPressed) in actions)
            PopupMenuItem(value: onPressed, child: ListTile(leading: Icon(icon), title: Text(label))),
        ],
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.store.selected;
    // A first message shown here once its agent has taken it, sent from another view of this session, goes.
    if (session?.firstSent case final sent? when _input.text == sent && !widget.store.isStarting(session!)) {
      session.firstSent = null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _input.text == sent) _input.clear();
      });
    }

    final conversation = Column(children: [
      Material(
        color: context.look.bar,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(children: [
            if (widget.narrow)
              Builder(builder: (inner) => IconButton(
                tooltip: 'Sessions',
                icon: const Icon(Icons.menu),
                onPressed: () => Scaffold.of(inner).openDrawer(),
              )),
            Expanded(child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(session?.title ?? 'Canoa', style: const TextStyle(
                  fontSize: 17, fontWeight: FontWeight.w600,
                ), maxLines: 1, overflow: TextOverflow.ellipsis),
                Text(_subtitle(session),
                  style: Theme.of(context).textTheme.bodySmall, maxLines: 1, overflow: TextOverflow.ellipsis),
              ],
            )),
            ..._headerActions(session),
            if (session != null)
              IconButton(
                tooltip: 'Close conversation',
                icon: const Icon(Icons.close),
                onPressed: () => unawaited((widget.handle.close ?? widget.store.close)(session)),
              ),
            // On a phone the state is an icon whose tooltip names it, which leaves the conversation's name room to be read.
            // The welcome page, with no session, has no state to name.
            if (session == null)
              const SizedBox.shrink()
            else if (widget.narrow)
              Tooltip(
                message: _state(session),
                child: const Padding(padding: EdgeInsets.all(8), child: Icon(Icons.hub_outlined, size: 18)),
              )
            else
              Chip(
                avatar: const Icon(Icons.hub_outlined, size: 16),
                label: Text(_state(session)),
                visualDensity: VisualDensity.compact,
              ),
          ]),
        ),
      ),
      if (session != null && widget.store.inBypass(session))
        chat.BypassBanner(onLeave: () => unawaited(widget.store.leaveBypass(session))),
      if (session == null)
        Expanded(child: widget.welcome)
      else if (session.isDraft)
        Expanded(
          child: _dropping(session, NewSessionView(
            store: widget.store,
            session: session,
            kit: widget.kit,
            composer: _composerFor(session, centred: true),
          )),
        )
      else ...[
        Expanded(
          child: _dropping(session, chat.ChatTranscript(
            key: _transcript,
            entries: session.entries,
            terminals: session.terminals,
            plan: session.plan,
            working: session.status == SessionStatus.working,
            renderers: widget.renderers,
            narrow: widget.narrow,
          )),
        ),
        if (session.queue.isNotEmpty) SessionQueuePanel(store: widget.store, session: session),
        if (session.pendingPermissions.isNotEmpty) SessionPermissionCard(store: widget.store, session: session),
        if (session.pendingQuestions.isNotEmpty) SessionQuestionCard(store: widget.store, session: session),
        if (session.pendingSecrets.isNotEmpty) SessionSecretCard(store: widget.store, session: session),
        chat.UsageFooter(text: chat.describeUsage(session.usage, spend: session.spend)),
        Container(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
          child: Center(
            heightFactor: 1,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.chatLook.messageWidth + 100),
              child: Column(mainAxisSize: MainAxisSize.min, children: [
                _composerFor(session),
                if (!canRunLocalAgents || session.place != SessionPlace.local) ...[
                  const SizedBox(height: 8),
                  Text('Remote sessions require verified peer authentication',
                    style: TextStyle(fontSize: 11, color: context.chatLook.muted)),
                ],
              ]),
            ),
          ),
        ),
      ],
    ]);
    if (widget.narrow || !_changesOpen || session == null || sessionChanges(session).isEmpty) return conversation;
    return Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Expanded(child: conversation),
      const VerticalDivider(width: 1),
      SizedBox(
        width: 420,
        child: SelectionArea(
          child: SessionChangesPane(
            store: widget.store,
            session: session,
            onComment: _addToComposer,
            onClose: () => setState(() => _changesOpen = false),
          ),
        ),
      ),
    ]);
  }

  /// The composer of [session]: in the middle of a new session's view when [centred], else docked under the conversation.
  Widget _composerFor(ChatSession session, {bool centred = false}) => SessionComposer(
    store: widget.store,
    session: session,
    controller: _input,
    focusNode: _composerFocus,
    kit: widget.kit,
    centred: centred,
    enabled: !widget.store.isStarting(session) && (session.status == SessionStatus.preview ||
        session.status == SessionStatus.draft ||
        session.status == SessionStatus.connected ||
        session.status == SessionStatus.working),
    onSend: _send,
    working: session.status == SessionStatus.working,
    onStop: () {
      final messenger = ScaffoldMessenger.of(context);
      unawaited(widget.store.cancelActiveSession().catchError((Object error) {
        messenger.showSnackBar(SnackBar(content: Text('Could not stop: $error')));
      }));
    },
    commands: session.commands,
  );

  /// [child], taking files dropped on it as attachments where this app can read them.
  Widget _dropping(ChatSession session, Widget child) {
    if (widget.kit.statFile == null || !widget.store.canAttach(session)) return child;
    return DropTarget(
      onDragDone: (details) async {
        final messenger = ScaffoldMessenger.of(context);
        final problems = await addPaths(widget.store, session, [
          for (final file in details.files)
            if (file.path.isNotEmpty) file.path,
        ], widget.kit);
        if (problems.isNotEmpty) messenger.showSnackBar(SnackBar(content: Text(problems.join('\n'))));
      },
      child: child,
    );
  }
}
