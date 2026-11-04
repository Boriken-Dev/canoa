import 'attention/notifier.dart';
import 'config/settings_dialog.dart';
import 'dart:async';
import 'config/canoa_config.dart';
import 'config/canoa_config_file.dart';
import 'desktop/desktop_shell.dart';
import 'desktop/host_supervisor.dart';
import 'desktop/host_tool.dart';
import 'desktop/hosting_dialog.dart';
import 'extension_context.dart';
import 'acp/browser_connect.dart';
import 'acp/connect.dart' show chatEndpoints;
import 'acp/provider_connect.dart';
import 'acp/agent_commands.dart';
import 'acp/client.dart';
import 'acp/local_agent.dart';
import 'package:coqui_acp/page.dart';
import 'connect_dialog.dart';
import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show KeyDownEvent, KeyRepeatEvent, LogicalKeyboardKey;

import 'chat/agent_events.dart' show AgentCommandInfo;
import 'chat/changes.dart';
import 'chat/palette.dart';
import 'chat/queue_panel.dart';
import 'chat/permission_card.dart';
import 'chat/transcript.dart';
import 'chat/update_log.dart';
import 'auth/peer_proof.dart' show PeerMethod;
import 'ledger/ledger_backend.dart';
import 'ledger/ledger_list.dart';
import 'ledger/session_ledger.dart';
import 'profile/profile_home_platform.dart';
import 'profile/profiles_dialog.dart';
import 'session_store.dart';
import 'theme/look.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final config = await loadConfig();
  // Beside Canoa.cfg on a desktop, in this origin's IndexedDB on the web.
  final ledger = SessionLedger(platformLedgerBackend(configPath: desktopConfigPath()), settings: config.sessions);
  await ledger.load();
  final shell = createDesktopShell();
  final settingsSignal = ValueNotifier<int>(0);
  final hostingSignal = ValueNotifier<int>(0);
  final host = shell.isDesktop ? HostSupervisor(findHostTool()) : null;
  await shell.initialize(
    config,
    onSettings: () => settingsSignal.value++,
    onHosting: host == null ? null : () => hostingSignal.value++,
  );
  runApp(CanoaApp(
    config: config, shell: shell, settingsSignal: settingsSignal,
    host: host, hostingSignal: hostingSignal, ledger: ledger, notifier: platformNotifier(),
  ));
  await host?.apply(configPath: desktopConfigPath(), hasEntries: config.hostEntries.isNotEmpty);
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal, this.host, this.hostingSignal,
    this.configPath, this.writeConfig, this.ledger, this.notifier,
  });

  final SessionStore? store;

  /// How a session that needs the person reaches them outside the window; none, nothing does.
  final Notifier? notifier;

  /// The list of sessions kept across runs, for a store this app makes; a given [store] brings its own.
  final SessionLedger? ledger;
  final CanoaConfig config;
  final DesktopShell? shell;
  final ValueNotifier<int>? settingsSignal;

  /// The `canoa-host` this app runs, on a desktop.
  final HostSupervisor? host;
  final ValueNotifier<int>? hostingSignal;

  /// Where Canoa.cfg is and how it is written; by default the platform's place.
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

  Future<void> _showHosting() async {
    final host = widget.host;
    if (host == null || _hostingOpen) return;
    final above = _navigator.currentContext;
    if (above == null) return;
    _hostingOpen = true;
    try {
      await showDialog<void>(
        context: above,
        builder: (_) => HostingDialog(supervisor: host, config: config, onSave: _saveHosting),
      );
    } finally {
      _hostingOpen = false;
    }
  }

  /// Saves Canoa.cfg once canoa-host finds nothing wrong with it, then serves it.
  /// Only [host] comes from the dialog: the rest is what the app has now, which Settings may have changed meanwhile.
  Future<String?> _saveHosting(CanoaConfig fromDialog) async {
    final host = widget.host;
    final path = _configPath;
    if (host == null || path == null) return 'Canoa.cfg has no place on this computer';
    final changed = config.withHost(fromDialog.host);
    if (changed.hostEntries.isNotEmpty) {
      final problem = await withFileBeside(path, changed.encode(), host.check);
      if (problem != null) return problem;
    }
    await _write(changed);
    if (mounted) setState(() => config = changed);
    await host.apply(configPath: path, hasEntries: changed.hostEntries.isNotEmpty);
    return null;
  }

  Future<void> _showSettings() async {
    final shell = widget.shell;
    final above = _navigator.currentContext;
    if (above == null || _settingsOpen) return;
    _settingsOpen = true;
    final CanoaConfig? fromDialog;
    try {
      fromDialog = await showDialog<CanoaConfig>(
        context: above,
        builder: (_) => SettingsDialog(
          config: config, onQuit: shell != null && shell.isDesktop ? shell.quit : null, ledger: store.ledger,
          onProfiles: _showProfiles, agents: [for (final agent in [...agentPresets, ...config.agents]) agent.label],
          onAllowNotifications: kIsWeb ? widget.notifier?.requestPermission : null,
        ),
      );
    } finally {
      _settingsOpen = false;
    }
    if (fromDialog == null || !mounted) return;
    // [host] and the profile tables stay as the app has them now: Hosting may have changed one, and an overlay installed
    // from Profiles is pinned in [overlays], while Settings was open.
    final changed = fromDialog.withHost(config.host).withProfilesOf(config);
    setState(() => config = changed);
    store.ledger?.configure(changed.sessions);
    for (final agent in {...store.fullAccessAgents, ...changed.fullAccess}) {
      store.setAgentFullAccess(agent, changed.fullAccess.contains(agent));
    }
    await widget.shell?.update(changed);
    try {
      await _write(changed);
    } on UnsupportedError {
      // Canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      if (mounted) {
        _messenger.currentState?.showSnackBar(
          SnackBar(content: Text('Could not save Canoa.cfg: $error')),
        );
      }
    }
  }
  /// Ends full access for [agent], in its sessions and in Canoa.cfg: the banner's one action, where the agent's setting turned it on.
  Future<void> _endAgentFullAccess(String agent) async {
    store.setAgentFullAccess(agent, false);
    if (!config.fullAccess.contains(agent)) return;
    final changed = config.copyWith(fullAccess: {...config.fullAccess}..remove(agent));
    setState(() => config = changed);
    try {
      await _write(changed);
    } on UnsupportedError {
      // Canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('Could not save Canoa.cfg: $error')));
    }
  }

  /// The profiles as Canoa.cfg has them now, for every agent the app offers or the file gives a profile.
  Future<void> _showProfiles() async {
    final above = _navigator.currentContext;
    if (above == null) return;
    await showDialog<void>(
      context: above,
      builder: (_) => ProfilesDialog(
        config: config,
        agents: {for (final agent in [...agentPresets, ...config.agents]) agent.label, ...config.agentProfiles.keys}
            .toList(),
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
      final client = await connectProviderAgent(acp, agent, decider: store.askPermission);
      try {
        final opened = await client.openSession(cwd: cwd);
        if (!mounted) throw StateError('Canoa UI closed');
        final local = store.addProviderSession(
          title: title.isEmpty ? agent.label : title,
          agent: agent.label,
          peerId: client.peerId,
          remoteSessionId: opened.id,
        );
        local.agentNames = title.isEmpty;
        store.applyOpened(local, opened);
        store.attachVerifiedClient(local.id, client);
        _ownedClients.add(client);
      } catch (_) {
        await client.close();
        rethrow;
      }
    }
  }

  /// An agent as a child of this app. Its permission requests go to the person, in the conversation.
  Future<void> _runLocalAgent(AgentCommand command, {required String cwd, required String title}) async {
    // The dialog's `/` is a remote agent's default. For one on this computer it would be the root of the filesystem.
    final where = cwd.isEmpty || cwd == '/' ? defaultWorkingDirectory() : cwd;
    final client = await startLocalAgent(
      command,
      decider: store.askPermission,
      workingDirectory: where,
    );
    try {
      final opened = await client.openSession(cwd: where ?? cwd);
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addProviderSession(
        title: title.isEmpty ? command.label : title,
        agent: command.label,
        cwd: where ?? cwd,
        peerId: client.peerId,
        remoteSessionId: opened.id,
      );
      local.agentNames = title.isEmpty;
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
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
    final VerifiedAgent client;
    switch (entry.place) {
      case SessionPlace.local:
        final command = [...agentPresets, ...config.agents].where((agent) => agent.label == entry.entry).firstOrNull;
        if (command == null) throw StateError('No agent named ${entry.entry} is set up here now');
        if (!canRunLocalAgents) throw StateError('This build cannot run agents on this computer');
        client = await startLocalAgent(command, decider: store.askPermission, workingDirectory: entry.cwd);
      case SessionPlace.host || SessionPlace.peer:
        final endpoint = Uri.tryParse(entry.endpoint ?? '');
        final method = PeerMethod.values.where((method) => method.id == entry.method).firstOrNull;
        if (endpoint == null || method == null) throw StateError('Canoa did not note how to reach that peer');
        final overWebTransport = endpoint.scheme == 'https';
        client = await connectBrowserAcp(
          chatEndpoints(
            webSocket: overWebTransport ? (entry.fallback ?? '') : endpoint.toString(),
            webTransport: overWebTransport ? endpoint.toString() : '',
            preferWebTransport: config.preferWebTransport,
          ),
          pinnedPeerId: entry.peerId, method: method,
          timeout: Duration(seconds: config.connectTimeout),
          decider: store.askPermission,
        );
      case SessionPlace.browser:
        throw StateError(why ?? 'Reopen it from Earlier conversations');
    }
    try {
      await store.reopen(entry, client);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// Ends every agent this app started, and the host with the agents it started.
  Future<void> _endAgents() async {
    await store.ledger?.flush();
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
    final client = await connectBrowserAcp(
      endpoints, pinnedPeerId: pinnedPeerId, method: target.method,
      timeout: Duration(seconds: config.connectTimeout),
      decider: store.askPermission,
    );
    try {
      final opened = await client.openSession(cwd: target.cwd);
      final remoteId = opened.id;
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addRemoteSession(
        title: target.title.isEmpty ? 'Remote agent' : target.title,
        agent: pinnedPeerId.substring(0, 22),
        peerId: pinnedPeerId,
        remoteSessionId: remoteId,
        endpoint: wt ?? ws,
        websocketFallback: wt == null ? null : ws,
        place: widget.host?.peerId?.toLowerCase() == pinnedPeerId ? SessionPlace.host : SessionPlace.peer,
        method: target.method.id,
        cwd: target.cwd,
      );
      local.agentNames = target.title.isEmpty;
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
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
        SessionStore(recorder: updateRecorderFromEnvironment(), ledger: widget.ledger, fullAccessAgents: config.fullAccess);
    // Closing a window to the tray keeps its agents. Quitting does not, and `dispose` does not run when the window is destroyed.
    widget.shell?.beforeQuit = _endAgents;
    _attention = store.attention.listen((needs) => unawaited(_notify(needs.$1, needs.$2)));
    unawaited(widget.notifier?.initialize(_openFromNotification));
    store.addListener(_showWaiting);
    unawaited(_claimContext());
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
    _contextTimer?.cancel();
    unawaited(_attention?.cancel());
    store.removeListener(_showWaiting);
    widget.settingsSignal?.removeListener(_requestSettings);
    widget.hostingSignal?.removeListener(_requestHosting);
    widget.shell?.beforeQuit = null;
    if (ownsStore) store.dispose();
    for (final client in _ownedClients) {
      unawaited(client.close());
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      navigatorKey: _navigator,
      scaffoldMessengerKey: _messenger,
      title: 'Canoa',
      debugShowCheckedModeBanner: false,
      theme: themeFor(config.look, Brightness.light),
      darkTheme: themeFor(config.look, Brightness.dark),
      themeMode: config.themeMode,
      home: AnimatedBuilder(
        animation: store,
        builder: (context, _) => _CanoaHome(
          store: store, onConnect: _connect, onUseBrowserAgents: _useBrowserAgents, onRunLocalAgent: canRunLocalAgents ? _runLocalAgent : null, onSettings: _showSettings,
          onHosting: widget.host == null ? null : _showHosting, onReopen: _reopen,
          agents: [...agentPresets, ...config.agents],
          onEndAgentFullAccess: _endAgentFullAccess,
          conversation: _conversation,
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onRunLocalAgent, required this.onSettings,
    this.agents = agentPresets, this.onHosting, this.onReopen, this.onEndAgentFullAccess, required this.conversation,
  });

  final List<AgentCommand> agents;

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;
  final Future<void> Function(LedgerEntry)? onReopen;
  final Future<void> Function(String agent)? onEndAgentFullAccess;
  final _ConversationHandle conversation;

  void _connect(BuildContext context) => unawaited(showCanoaConnectDialog(
    context,
    onConnect,
    browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
    onUseBrowserAgents: onUseBrowserAgents,
    onRunLocalAgent: onRunLocalAgent,
    localAgents: agents,
  ));

  /// The session [step] places from the one chosen, going round the list.
  void _step(int step) {
    final sessions = store.sessions;
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

  Future<void> _palette(BuildContext context) {
    final session = store.selected;
    return showDialog<void>(
      context: context,
      builder: (_) => CommandPalette(items: [
        PaletteItem('New session', store.createPreviewSession, icon: Icons.add_comment_outlined),
        PaletteItem('Connect to an agent', () => _connect(context), icon: Icons.link_rounded),
        if (session != null && conversation.showChanges != null && changesOf(session).isNotEmpty)
          PaletteItem('Show the changes', conversation.showChanges!, icon: Icons.difference_outlined),
        if (session?.status == SessionStatus.working) PaletteItem('Stop the turn', _stop, icon: Icons.stop_circle_outlined),
        PaletteItem('Settings', onSettings, icon: Icons.settings_outlined),
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
    chord(LogicalKeyboardKey.keyN): store.createPreviewSession,
    for (final (index, key) in const [
      LogicalKeyboardKey.digit1, LogicalKeyboardKey.digit2, LogicalKeyboardKey.digit3,
      LogicalKeyboardKey.digit4, LogicalKeyboardKey.digit5, LogicalKeyboardKey.digit6,
      LogicalKeyboardKey.digit7, LogicalKeyboardKey.digit8, LogicalKeyboardKey.digit9,
    ].indexed)
      chord(key): () {
        if (index < store.sessions.length) store.select(store.sessions[index].id);
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

      return Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings, onHosting: onHosting, onReopen: onReopen, agents: agents)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: context.look.railWidth,
                  child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings, onHosting: onHosting, onReopen: onReopen, agents: agents),
                ),
              Expanded(
                child: _Conversation(
                  key: ValueKey(store.selectedId ?? 'empty'),
                  store: store,
                  narrow: narrow,
                  onEndAgentFullAccess: onEndAgentFullAccess,
                  handle: conversation,
                ),
              ),
            ],
          ),
        ),
      );
    });
  }
}

class _SessionRail extends StatelessWidget {
  const _SessionRail({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onRunLocalAgent, required this.onSettings,
    this.agents = agentPresets, this.onHosting, this.onReopen,
  });

  final List<AgentCommand> agents;

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;
  final Future<void> Function(LedgerEntry)? onReopen;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: context.look.rail,
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 12),
          // The wordmark never wraps: where it and the buttons do not fit on one line, the buttons go under it. The full width puts
          // them at the right end when they do fit.
          child: SizedBox(width: double.infinity, child: Wrap(
            alignment: WrapAlignment.spaceBetween,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Row(mainAxisSize: MainAxisSize.min, children: [
                Icon(Icons.sailing_rounded, color: context.look.accent),
                const SizedBox(width: 10),
                const Text('Canoa', softWrap: false, style: TextStyle(
                  fontSize: 22, fontWeight: FontWeight.w700,
                )),
              ]),
              Row(mainAxisSize: MainAxisSize.min, children: [
                IconButton(
                  tooltip: 'Canoa settings',
                  onPressed: onSettings,
                  icon: const Icon(Icons.settings_outlined),
                ),
                if (onHosting != null)
                  IconButton(
                    tooltip: 'Hosting',
                    onPressed: onHosting,
                    icon: const Icon(Icons.dns_outlined),
                  ),
                IconButton(
                  tooltip: 'Connect to verified agent',
                  onPressed: () => showCanoaConnectDialog(
                    context,
                    onConnect,
                    browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
                    onUseBrowserAgents: onUseBrowserAgents,
                    onRunLocalAgent: onRunLocalAgent,
                    localAgents: agents,
                  ),
                  icon: const Icon(Icons.link_rounded),
                ),
                IconButton(
                  tooltip: 'New session',
                  onPressed: () {
                    store.createPreviewSession();
                    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) {
                      Navigator.pop(context);
                    }
                  },
                  icon: const Icon(Icons.add_comment_outlined),
                ),
              ]),
            ],
          )),
        ),
        const Divider(height: 1),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.symmetric(vertical: 8),
            itemCount: store.sessions.length + (store.ledger == null ? 0 : 1),
            itemBuilder: (context, index) {
              // The open conversations, then the ledger's sessions that are not open now, by harness.
              if (index == store.sessions.length) {
                return LedgerList(ledger: store.ledger!, store: store, onReopen: onReopen);
              }
              final session = store.sessions[index];
              final active = session.id == store.selectedId;
              return Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                child: ListTile(
                  selected: active,
                  selectedTileColor: context.look.selected,
                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                  leading: session.fullAccess == null
                      ? const Icon(Icons.chat_bubble_outline, size: 21)
                      : Icon(Icons.gpp_maybe_outlined, size: 21, color: Theme.of(context).colorScheme.error),
                  title: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                  trailing: _SessionMark(session: session),
                  subtitle: Text(
                    [
                      if (session.fullAccess != null) 'Full access',
                      session.status == SessionStatus.preview ? 'Preview · offline' : session.status.name,
                    ].join(' · '),
                    maxLines: 1,
                  ),
                  onTap: () {
                    store.select(session.id);
                    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) {
                      Navigator.pop(context);
                    }
                  },
                ),
              );
            },
          ),
        ),
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(
            // An agent this app runs is its own child, and only one elsewhere is reached as a peer.
            canRunLocalAgents
                ? 'Agents on other machines need a verified ACP peer.'
                : 'Agent connections and sync require verified ACP peers.',
            style: TextStyle(color: context.look.muted, fontSize: 12),
          ),
        ),
      ]),
    );
  }
}

/// What a session in the list has for the person: something waiting for their answer, a turn running, or a turn that ended
/// since they last looked.
class _SessionMark extends StatelessWidget {
  const _SessionMark({required this.session});

  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    if (session.needsYou) {
      return Tooltip(message: 'Needs you', child: Icon(Icons.front_hand_outlined, size: 18, color: scheme.error));
    }
    if (session.status == SessionStatus.working) {
      return const Tooltip(
        message: 'Working',
        child: SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2)),
      );
    }
    if (session.unread) {
      return Tooltip(
        message: 'New since you looked',
        child: Container(width: 10, height: 10, decoration: BoxDecoration(color: scheme.primary, shape: BoxShape.circle)),
      );
    }
    return const SizedBox.shrink();
  }
}

/// What the conversation shown offers the keyboard and the palette, set by it while it is shown.
final class _ConversationHandle {
  Object? owner;
  VoidCallback? focusComposer, showChanges;
  ValueChanged<String>? insert;
}

class _Conversation extends StatefulWidget {
  const _Conversation({
    super.key, required this.store, required this.narrow, this.onEndAgentFullAccess, required this.handle,
  });

  final _ConversationHandle handle;

  final SessionStore store;
  final bool narrow;

  /// Ends full access for an agent, where its setting is what turned it on.
  final Future<void> Function(String agent)? onEndAgentFullAccess;

  @override
  State<_Conversation> createState() => _ConversationState();
}

class _ConversationState extends State<_Conversation> {
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();

  /// The composer's focus. Up in an empty composer brings back the last message the person sent here; otherwise Up moves the
  /// cursor as usual, since the composer's own handler sees the key before the text field's shortcuts do.
  late final FocusNode _composerFocus = FocusNode(onKeyEvent: (node, event) {
    if ((event is KeyDownEvent || event is KeyRepeatEvent) && event.logicalKey == LogicalKeyboardKey.arrowUp &&
        _input.text.isEmpty) {
      final last = widget.store.selected?.messages.lastWhere((message) => message.role == MessageRole.user,
          orElse: () => ChatMessage(role: MessageRole.user, text: '', at: DateTime.now()));
      if (last != null && last.text.isNotEmpty) {
        _input.value = TextEditingValue(text: last.text, selection: TextSelection.collapsed(offset: last.text.length));
        return KeyEventResult.handled;
      }
    }
    return KeyEventResult.ignored;
  });

  @override
  void initState() {
    super.initState();
    widget.handle
      ..owner = this
      ..focusComposer = _composerFocus.requestFocus
      ..showChanges = () {
        final session = widget.store.selected;
        if (session != null && changesOf(session).isNotEmpty) _showChanges(session);
      }
      ..insert = (text) {
        _input.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
        _composerFocus.requestFocus();
      };
  }

  /// Whether the transcript keeps its end in view as entries arrive: true until the person scrolls away from the end.
  bool _following = true;

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
      builder: (sheet) => FractionallySizedBox(
        heightFactor: 0.9,
        child: ListenableBuilder(
          listenable: widget.store,
          builder: (context, _) => ChangesPane(
            store: widget.store,
            session: session,
            onComment: (text) {
              _addToComposer(text);
              Navigator.pop(sheet);
            },
            onClose: () => Navigator.pop(sheet),
          ),
        ),
      ),
    ));
  }

  /// How far from the end, in logical pixels, still counts as at the end.
  static const double _nearEnd = 24;

  bool _onScroll(ScrollNotification notification) {
    // Only the transcript's own scrolling, not a code box's inside an entry.
    if (notification.depth != 0 || (notification is! ScrollUpdateNotification && notification is! ScrollEndNotification)) {
      return false;
    }
    final atEnd = notification.metrics.extentAfter <= _nearEnd;
    if (atEnd != _following) setState(() => _following = atEnd);
    return false;
  }

  /// The transcript grew or shrank: while following, its end comes into view.
  bool _onMetrics(ScrollMetricsNotification notification) {
    if (notification.depth == 0 && _following && notification.metrics.extentAfter > 0) _toEnd();
    return false;
  }

  void _toEnd() {
    if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
  }

  void _jumpToLatest() {
    setState(() => _following = true);
    _toEnd();
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
    _composerFocus.dispose();
    _input.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _attach(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final chosen = await openFiles();
    for (final file in chosen) {
      final problem = widget.store.addAttachment(session, file.name, await file.readAsBytes(), mimeType: file.mimeType);
      if (problem != null) messenger.showSnackBar(SnackBar(content: Text(problem)));
    }
  }

  Future<void> _showEarlier(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final picked = await showDialog<AcpSessionInfo>(
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
    final attached = widget.store.selected?.attachments.isNotEmpty ?? false;
    if (text.trim().isEmpty && !attached) return;
    try {
      _input.clear();
      unawaited(widget.store.sendText(text).catchError((Object error) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Message not accepted: $error')),
          );
        }
      }));
    } on StateError {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
        content: Text('Connect and verify an ACP peer before sending.'),
      ));
    }
    setState(() => _following = true);
    WidgetsBinding.instance.addPostFrameCallback((_) => _toEnd());
  }

  /// What the header offers for [session], beside closing it: as buttons, or under one menu where two or more would crowd a phone.
  List<Widget> _headerActions(ChatSession? session) {
    final actions = <(String, IconData, VoidCallback)>[
      if (session != null && widget.store.canListEarlier(session))
        ('Earlier conversations', Icons.history, () => unawaited(_showEarlier(session))),
      if (session != null && session.status != SessionStatus.preview && session.fullAccess == null)
        ('Full access…', Icons.gpp_maybe_outlined, () async {
          if (await confirmFullAccess(context, session)) widget.store.setFullAccess(session, true);
        }),
      if (session != null && changesOf(session).isNotEmpty) ('Changes', Icons.difference_outlined, () => _showChanges(session)),
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
                Text(session?.agent ?? 'Multi-session agent chat',
                  style: Theme.of(context).textTheme.bodySmall, maxLines: 1, overflow: TextOverflow.ellipsis),
              ],
            )),
            ..._headerActions(session),
            if (session != null)
              IconButton(
                tooltip: 'Close conversation',
                icon: const Icon(Icons.close),
                onPressed: () => unawaited(widget.store.close(session)),
              ),
            // On a phone the state is an icon whose tooltip names it, which leaves the conversation's name room to be read.
            if (widget.narrow)
              Tooltip(
                message: session?.status == SessionStatus.preview ? 'Preview' : (session?.status.name ?? 'Offline'),
                child: const Padding(padding: EdgeInsets.all(8), child: Icon(Icons.hub_outlined, size: 18)),
              )
            else
              Chip(
                avatar: const Icon(Icons.hub_outlined, size: 16),
                label: Text(session?.status == SessionStatus.preview
                    ? 'Preview' : (session?.status.name ?? 'Offline')),
                visualDensity: VisualDensity.compact,
              ),
          ]),
        ),
      ),
      if (session?.fullAccess != null)
        FullAccessBanner(
          session: session!,
          onTurnOff: () {
            final agent = session.fullAccess == FullAccess.agent ? widget.onEndAgentFullAccess : null;
            if (agent != null) {
              unawaited(agent(session.agent));
            } else if (session.fullAccess == FullAccess.agent) {
              widget.store.setAgentFullAccess(session.agent, false);
            } else {
              widget.store.setFullAccess(session, false);
            }
          },
        ),
      if (session == null)
        const Expanded(child: Center(child: Text('Create a conversation to begin.')))
      else ...[
        Expanded(
          child: Stack(children: [
            NotificationListener<ScrollMetricsNotification>(
              onNotification: _onMetrics,
              child: NotificationListener<ScrollNotification>(
                onNotification: _onScroll,
                child: ListView.builder(
                  controller: _scroll,
                  padding: EdgeInsets.symmetric(
                    horizontal: widget.narrow ? 12 : 40,
                    vertical: context.look.padding + 10,
                  ),
                  itemCount: session.messages.length,
                  itemBuilder: (context, index) {
                    return TranscriptEntry(message: session.messages[index], session: session);
                  },
                ),
              ),
            ),
            if (!_following)
              Positioned(
                left: 0,
                right: 0,
                bottom: 12,
                child: Center(
                  child: IconButton.filledTonal(
                    tooltip: 'Jump to latest',
                    onPressed: _jumpToLatest,
                    icon: const Icon(Icons.arrow_downward_rounded),
                  ),
                ),
              ),
          ]),
        ),
        if (session.plan.isNotEmpty) PlanPanel(entries: session.plan),
        if (session.queue.isNotEmpty) QueuePanel(store: widget.store, session: session),
        if (session.pendingPermissions.isNotEmpty)
          PermissionCard(store: widget.store, session: session),
        SessionControls(
          session: session,
          onChoose: (option, value) => unawaited(widget.store.choose(session, option, value)),
        ),
        _Composer(
          controller: _input,
          focusNode: _composerFocus,
          enabled: session.status == SessionStatus.preview ||
              session.status == SessionStatus.connected ||
              session.status == SessionStatus.working,
          onSend: _send,
          working: session.status == SessionStatus.working,
          onStop: () {
            final messenger = ScaffoldMessenger.of(context);
            unawaited(widget.store.cancelActiveSession().catchError((Object error) {
              messenger.showSnackBar(SnackBar(content: Text('Could not stop: $error')));
            }));
          },
          commands: session.commands,
          attachments: session.attachments,
          onAttach: widget.store.canAttach(session) ? () => unawaited(_attach(session)) : null,
          onRemove: (attachment) => widget.store.removeAttachment(session, attachment),
          remote: !canRunLocalAgents || session.place != SessionPlace.local,
        ),
      ],
    ]);
    if (widget.narrow || !_changesOpen || session == null || changesOf(session).isEmpty) return conversation;
    return Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Expanded(child: conversation),
      const VerticalDivider(width: 1),
      SizedBox(
        width: 420,
        child: ChangesPane(
          store: widget.store,
          session: session,
          onComment: _addToComposer,
          onClose: () => setState(() => _changesOpen = false),
        ),
      ),
    ]);
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller, required this.enabled, required this.onSend, this.focusNode,
    this.working = false, this.onStop, this.commands = const [],
    this.attachments = const [], this.onAttach, this.onRemove, this.remote = true,
  });

  /// Whether the agent is reached as a peer, which is what the line under the field is about.
  final bool remote;

  final TextEditingController controller;
  final FocusNode? focusNode;
  final bool enabled;
  final VoidCallback onSend;
  final bool working;
  final VoidCallback? onStop;
  final List<AgentCommandInfo> commands;
  final List<AcpAttachment> attachments;
  final VoidCallback? onAttach;
  final ValueChanged<AcpAttachment>? onRemove;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Container(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
      child: Center(
        heightFactor: 1,
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Column(mainAxisSize: MainAxisSize.min, children: [
            ValueListenableBuilder<TextEditingValue>(
              valueListenable: controller,
              builder: (context, value, _) => CommandSuggestions(
                commands: commands,
                typed: value.text,
                onPick: (command) {
                  final text = '/${command.name} ';
                  controller.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
                },
              ),
            ),
            const SizedBox(height: 4),
            if (attachments.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(bottom: 6),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Wrap(spacing: 6, runSpacing: 4, children: [
                    for (final attachment in attachments)
                      InputChip(
                        avatar: Icon(attachment.isImage ? Icons.image_outlined : Icons.description_outlined, size: 16),
                        label: Text(attachment.name, overflow: TextOverflow.ellipsis),
                        onDeleted: () => onRemove?.call(attachment),
                      ),
                  ]),
                ),
              ),
            TextField(
              controller: controller,
              focusNode: focusNode,
              enabled: enabled,
              minLines: 1,
              maxLines: 5,
              textInputAction: TextInputAction.send,
              onSubmitted: (_) => onSend(),
              decoration: InputDecoration(
                hintText: enabled ? 'Message agent' : 'Connect an ACP peer to chat',
                filled: true,
                fillColor: look.composer,
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(look.radius + 2),
                  borderSide: BorderSide.none,
                ),
                prefixIcon: onAttach == null
                    ? null
                    : IconButton(tooltip: 'Attach', onPressed: enabled ? onAttach : null, icon: const Icon(Icons.attach_file)),
                // While the agent works, what is written waits in the queue, and Stop leaves the queue as it is.
                suffixIcon: working
                    ? Row(mainAxisSize: MainAxisSize.min, children: [
                        IconButton(tooltip: 'Queue', onPressed: enabled ? onSend : null, icon: const Icon(Icons.schedule_send_outlined)),
                        IconButton(tooltip: 'Stop', onPressed: onStop, icon: const Icon(Icons.stop_circle_outlined)),
                      ])
                    : IconButton(
                        tooltip: 'Send',
                        onPressed: enabled ? onSend : null,
                        icon: const Icon(Icons.arrow_upward_rounded),
                      ),
              ),
            ),
            if (remote) ...[
              const SizedBox(height: 8),
              Text('Remote sessions require verified peer authentication',
                style: TextStyle(fontSize: 11, color: look.muted)),
            ],
          ]),
        ),
      ),
    );
  }
}
