import 'config/settings_dialog.dart';
import 'dart:async';
import 'config/canoa_config.dart';
import 'config/canoa_config_file.dart';
import 'desktop/desktop_shell.dart';
import 'extension_context.dart';
import 'acp/browser_connect.dart';
import 'acp/provider_connect.dart';
import 'acp/agent_commands.dart';
import 'acp/client.dart';
import 'acp/local_agent.dart';
import 'package:coqui_acp/page.dart';
import 'connect_dialog.dart';
import 'package:coqui_web/coqui_web.dart';
import 'package:flutter/material.dart';

import 'chat/agent_events.dart' show AgentCommandInfo;
import 'chat/transcript.dart';
import 'session_store.dart';
import 'theme/look.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final config = await loadConfig();
  final shell = createDesktopShell();
  final settingsSignal = ValueNotifier<int>(0);
  await shell.initialize(config, onSettings: () => settingsSignal.value++);
  runApp(CanoaApp(
    config: config, shell: shell, settingsSignal: settingsSignal,
  ));
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal,
  });

  final SessionStore? store;
  final CanoaConfig config;
  final DesktopShell? shell;
  final ValueNotifier<int>? settingsSignal;

  @override
  State<CanoaApp> createState() => _CanoaAppState();
}

class _CanoaAppState extends State<CanoaApp> {
  late final SessionStore store;
  late final bool ownsStore;
  late CanoaConfig config;
  void _requestSettings() {
    if (!mounted) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _showSettings();
    });
  }
  Future<void> _showSettings() async {
    final changed = await showDialog<CanoaConfig>(
      context: context,
      builder: (_) => SettingsDialog(config: config),
    );
    if (changed == null || !mounted) return;
    setState(() => config = changed);
    await widget.shell?.update(changed);
    try {
      await saveConfig(changed);
    } on UnsupportedError {
      // Canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not save Canoa.cfg: $error')),
        );
      }
    }
  }
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
    final client = await startLocalAgent(
      command,
      decider: store.askPermission,
      workingDirectory: cwd.isEmpty || cwd == '/' ? null : cwd,
    );
    try {
      final opened = await client.openSession(cwd: cwd);
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addProviderSession(
        title: title.isEmpty ? command.label : title,
        agent: command.label,
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

  Future<void> _connect(ConnectTarget target) async {
    final ws = Uri.parse(target.webSocket);
    final wt = target.webTransport.isEmpty
        ? null : Uri.parse(target.webTransport);
    final endpoints = WebEndpoints(
      webTransport: config.preferWebTransport ? wt : null,
      webSocket: ws,
    );
    final pinnedPeerId = target.peerId.toLowerCase();
    final client = await connectBrowserAcp(
      endpoints, pinnedPeerId: pinnedPeerId,
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
    store = widget.store ?? SessionStore();
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
    if (ownsStore) store.dispose();
    for (final client in _ownedClients) {
      unawaited(client.close());
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Canoa',
      debugShowCheckedModeBanner: false,
      theme: themeFor(config.look, Brightness.light),
      darkTheme: themeFor(config.look, Brightness.dark),
      themeMode: config.themeMode,
      home: AnimatedBuilder(
        animation: store,
        builder: (context, _) => _CanoaHome(
          store: store, onConnect: _connect, onUseBrowserAgents: _useBrowserAgents, onRunLocalAgent: canRunLocalAgents ? _runLocalAgent : null, onSettings: _showSettings,
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onRunLocalAgent, required this.onSettings,
  });

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = constraints.maxWidth < 730 ||
          Uri.base.queryParameters['view'] == 'panel';

      return Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: context.look.railWidth,
                  child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onRunLocalAgent: onRunLocalAgent, onSettings: onSettings),
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
  });

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final RunLocalAgent? onRunLocalAgent;
  final VoidCallback onSettings;

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
            IconButton(
              tooltip: 'Connect to verified agent',
              onPressed: () => showCanoaConnectDialog(
                context,
                onConnect,
                browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
                onUseBrowserAgents: onUseBrowserAgents,
                onRunLocalAgent: onRunLocalAgent,
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

  void _send() {
    final text = _input.text;
    if (text.trim().isEmpty) return;
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
        ),
      ],
    ]);
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller, required this.enabled, required this.onSend,
    this.working = false, this.onStop, this.commands = const [],
  });

  final TextEditingController controller;
  final bool enabled;
  final VoidCallback onSend;
  final bool working;
  final VoidCallback? onStop;
  final List<AgentCommandInfo> commands;

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
