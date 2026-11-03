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
import 'package:flutter/material.dart';

import 'chat/agent_events.dart' show AgentCommandInfo, isLongToApprove;
import 'chat/transcript.dart';
import 'chat/update_log.dart';
import 'session_store.dart';
import 'theme/look.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final config = await loadConfig();
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
    host: host, hostingSignal: hostingSignal,
  ));
  await host?.apply(configPath: desktopConfigPath(), hasEntries: config.hostEntries.isNotEmpty);
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal, this.host, this.hostingSignal,
    this.configPath, this.writeConfig,
  });

  final SessionStore? store;
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
        builder: (_) => SettingsDialog(config: config, onQuit: shell != null && shell.isDesktop ? shell.quit : null),
      );
    } finally {
      _settingsOpen = false;
    }
    if (fromDialog == null || !mounted) return;
    // [host] stays as the app has it now: Hosting may have changed it while Settings was open.
    final changed = fromDialog.withHost(config.host);
    setState(() => config = changed);
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
  /// This state sits above the app's Navigator, so its dialogs open in the Navigator's context.
  final _navigator = GlobalKey<NavigatorState>();
  final _messenger = GlobalKey<ScaffoldMessengerState>();
  Timer? _contextTimer;
  final List<VerifiedAgent> _ownedClients = [];
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

  /// Ends every agent this app started, and the host with the agents it started.
  Future<void> _endAgents() async {
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
    store = widget.store ?? SessionStore(recorder: updateRecorderFromEnvironment());
    // Closing a window to the tray keeps its agents. Quitting does not, and `dispose` does not run when the window is destroyed.
    widget.shell?.beforeQuit = _endAgents;
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
          onHosting: widget.host == null ? null : _showHosting,
          agents: [...agentPresets, ...config.agents],
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onRunLocalAgent, required this.onSettings,
    this.agents = agentPresets, this.onHosting,
  });

  final List<AgentCommand> agents;

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = constraints.maxWidth < 730 ||
          Uri.base.queryParameters['view'] == 'panel';

      return Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings, onHosting: onHosting, agents: agents)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: context.look.railWidth,
                  child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings, onHosting: onHosting, agents: agents),
                ),
              Expanded(
                child: _Conversation(
                  key: ValueKey(store.selectedId ?? 'empty'),
                  store: store,
                  narrow: narrow,
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
    this.agents = agentPresets, this.onHosting,
  });

  final List<AgentCommand> agents;

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: context.look.rail,
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 12),
          child: Row(children: [
            Icon(Icons.sailing_rounded, color: context.look.accent),
            const SizedBox(width: 10),
            const Expanded(
              child: Text('Canoa', style: TextStyle(
                fontSize: 22, fontWeight: FontWeight.w700,
              )),
            ),
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
        ),
        const Divider(height: 1),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.symmetric(vertical: 8),
            itemCount: store.sessions.length,
            itemBuilder: (context, index) {
              final session = store.sessions[index];
              final active = session.id == store.selectedId;
              return Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                child: ListTile(
                  selected: active,
                  selectedTileColor: context.look.selected,
                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                  leading: const Icon(Icons.chat_bubble_outline, size: 21),
                  title: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                  subtitle: Text(
                    session.status == SessionStatus.preview ? 'Preview · offline' : session.status.name,
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
            'Agent connections and sync require verified ACP peers.',
            style: TextStyle(color: context.look.muted, fontSize: 12),
          ),
        ),
      ]),
    );
  }
}

class _Conversation extends StatefulWidget {
  const _Conversation({
    super.key, required this.store, required this.narrow,
  });

  final SessionStore store;
  final bool narrow;

  @override
  State<_Conversation> createState() => _ConversationState();
}

class _ConversationState extends State<_Conversation> {
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();

  @override
  void dispose() {
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
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.store.selected;

    return Column(children: [
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
                  style: Theme.of(context).textTheme.bodySmall),
              ],
            )),
            if (session != null && widget.store.canListEarlier(session))
              IconButton(
                tooltip: 'Earlier conversations',
                icon: const Icon(Icons.history),
                onPressed: () => unawaited(_showEarlier(session)),
              ),
            Chip(
              avatar: const Icon(Icons.hub_outlined, size: 16),
              label: Text(session?.status == SessionStatus.preview
                  ? 'Preview' : (session?.status.name ?? 'Offline')),
              visualDensity: VisualDensity.compact,
            ),
          ]),
        ),
      ),
      if (session == null)
        const Expanded(child: Center(child: Text('Create a conversation to begin.')))
      else ...[
        Expanded(
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
        if (session.plan.isNotEmpty) PlanPanel(entries: session.plan),
        if (session.pendingPermissions.isNotEmpty)
          _PermissionCard(store: widget.store, session: session),
        SessionControls(
          session: session,
          onChoose: (option, value) => unawaited(widget.store.choose(session, option, value)),
        ),
        _Composer(
          controller: _input,
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
        ),
      ],
    ]);
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller, required this.enabled, required this.onSend,
    this.working = false, this.onStop, this.commands = const [],
    this.attachments = const [], this.onAttach, this.onRemove,
  });

  final TextEditingController controller;
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
                suffixIcon: working
                    ? IconButton(tooltip: 'Stop', onPressed: onStop, icon: const Icon(Icons.stop_circle_outlined))
                    : IconButton(
                        tooltip: 'Send',
                        onPressed: enabled ? onSend : null,
                        icon: const Icon(Icons.arrow_upward_rounded),
                      ),
              ),
            ),
            const SizedBox(height: 8),
            Text('Remote sessions require verified peer authentication',
              style: TextStyle(fontSize: 11, color: look.muted)),
          ]),
        ),
      ),
    );
  }
}

/// What an agent is waiting on. It stays until the person answers: nothing answers it for them.
class _PermissionCard extends StatelessWidget {
  const _PermissionCard({required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final question = session.pendingPermissions.first;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Card(
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            children: [
              const Icon(Icons.shield_outlined),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('${session.agent} wants to use a tool', style: Theme.of(context).textTheme.labelMedium),
                    Text(question.title),
                    if (question.detail?.isNotEmpty ?? false)
                      Padding(
                        padding: const EdgeInsets.only(top: 4),
                        child: ConstrainedBox(
                          constraints: const BoxConstraints(maxHeight: 120),
                          child: SingleChildScrollView(
                            child: SelectableText(
                              question.detail!,
                              style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12),
                            ),
                          ),
                        ),
                      ),
                    if (question.detail != null && isLongToApprove(question.detail!))
                      Padding(
                        padding: const EdgeInsets.only(top: 4),
                        child: Text(
                          'This is long. Scroll to read all of it before you answer.',
                          style: Theme.of(context).textTheme.labelSmall?.copyWith(fontWeight: FontWeight.w600),
                        ),
                      ),
                    if (session.pendingPermissions.length > 1)
                      Text(
                        '${session.pendingPermissions.length - 1} more after this',
                        style: Theme.of(context).textTheme.labelSmall,
                      ),
                  ],
                ),
              ),
              TextButton(
                onPressed: () => store.answerPermission(session, allow: false),
                child: const Text('Refuse'),
              ),
              const SizedBox(width: 8),
              FilledButton(
                onPressed: () => store.answerPermission(session, allow: true),
                child: const Text('Allow'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
