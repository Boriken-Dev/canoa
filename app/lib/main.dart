import 'dart:async';
import 'extension_context.dart';
import 'acp/browser_connect.dart';
import 'acp/client.dart';
import 'connect_dialog.dart';
import 'transport/agent_transport.dart';
import 'package:flutter/material.dart';

import 'session_store.dart';

void main() {
  runApp(const CanoaApp());
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({super.key, this.store});

  final SessionStore? store;

  @override
  State<CanoaApp> createState() => _CanoaAppState();
}

class _CanoaAppState extends State<CanoaApp> {
  late final SessionStore store;
  late final bool ownsStore;
  Timer? _contextTimer;
  final List<AcpV2Client> _ownedClients = [];
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

  Future<void> _connect(ConnectTarget target) async {
    final ws = Uri.parse(target.webSocket);
    final wt = target.webTransport.isEmpty
        ? null : Uri.parse(target.webTransport);
    final endpoints = AgentEndpoints(webTransport: wt, webSocket: ws);
    final pinnedPeerId = target.peerId.toLowerCase();
    final client = await connectBrowserAcp(
      endpoints, pinnedPeerId: pinnedPeerId,
    );
    try {
      final remoteId = await client.createSession(cwd: target.cwd);
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addRemoteSession(
        title: target.title.isEmpty ? 'Remote agent' : target.title,
        agent: pinnedPeerId.substring(0, 22),
        peerId: pinnedPeerId,
        remoteSessionId: remoteId,
        endpoint: wt ?? ws,
        websocketFallback: wt == null ? null : ws,
      );
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
    store = widget.store ?? SessionStore();
    unawaited(_claimContext());
    _contextTimer = Timer.periodic(
      const Duration(seconds: 2), (_) => unawaited(_claimContext()),
    );
  }

  @override
  void dispose() {
    _contextTimer?.cancel();
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
      theme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.dark,
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF58D0C1),
          brightness: Brightness.dark,
        ),
        scaffoldBackgroundColor: const Color(0xFF101820),
      ),
      home: AnimatedBuilder(
        animation: store,
        builder: (context, _) => _CanoaHome(store: store, onConnect: _connect),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({required this.store, required this.onConnect});

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = constraints.maxWidth < 730 ||
          Uri.base.queryParameters['view'] == 'panel';

      return Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _SessionRail(store: store, onConnect: onConnect)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: 272,
                  child: _SessionRail(store: store, onConnect: onConnect),
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
  const _SessionRail({required this.store, required this.onConnect});

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: const Color(0xFF17242D),
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 12),
          child: Row(children: [
            const Icon(Icons.sailing_rounded, color: Color(0xFF6DE2D2)),
            const SizedBox(width: 10),
            const Expanded(
              child: Text('Canoa', style: TextStyle(
                fontSize: 22, fontWeight: FontWeight.w700,
              )),
            ),
            IconButton(
              tooltip: 'Connect to verified agent',
              onPressed: () => showCanoaConnectDialog(context, onConnect),
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
                  selectedTileColor: const Color(0xFF294952),
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
        const Padding(
          padding: EdgeInsets.all(16),
          child: Text(
            'Agent connections and sync require verified ACP peers.',
            style: TextStyle(color: Color(0xFFABBEC8), fontSize: 12),
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
        color: const Color(0xFF15222C),
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
              vertical: 24,
            ),
            itemCount: session.messages.length,
            itemBuilder: (context, index) {
              final message = session.messages[index];
              return _MessageBubble(message: message);
            },
          ),
        ),
        _Composer(
          controller: _input,
          enabled: session.status == SessionStatus.preview ||
              session.status == SessionStatus.connected ||
              session.status == SessionStatus.working,
          onSend: _send,
        ),
      ],
    ]);
  }
}

class _MessageBubble extends StatelessWidget {
  const _MessageBubble({required this.message});

  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    final fromUser = message.role == MessageRole.user;
    final notice = message.role == MessageRole.notice;
    return Align(
      alignment: fromUser ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 720),
        child: Container(
          margin: const EdgeInsets.only(bottom: 14),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          decoration: BoxDecoration(
            color: notice
                ? const Color(0xFF26333D)
                : fromUser ? const Color(0xFF235853) : const Color(0xFF1D2B34),
            borderRadius: BorderRadius.circular(16),
          ),
          child: Text(message.text, style: TextStyle(
            fontSize: notice ? 13 : 15,
            color: notice ? const Color(0xFFBDD0D6) : null,
            height: 1.45,
          )),
        ),
      ),
    );
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller, required this.enabled, required this.onSend,
  });

  final TextEditingController controller;
  final bool enabled;
  final VoidCallback onSend;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
      child: Center(
        heightFactor: 1,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 820),
          child: Column(mainAxisSize: MainAxisSize.min, children: [
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
                fillColor: const Color(0xFF1B3039),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(18),
                  borderSide: BorderSide.none,
                ),
                suffixIcon: IconButton(
                  tooltip: 'Send',
                  onPressed: enabled ? onSend : null,
                  icon: const Icon(Icons.arrow_upward_rounded),
                ),
              ),
            ),
            const SizedBox(height: 8),
            const Text('Remote sessions require verified peer authentication',
              style: TextStyle(fontSize: 11, color: Color(0xFF9CB3BD))),
          ]),
        ),
      ),
    );
  }
}
