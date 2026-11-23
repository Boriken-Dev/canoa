import 'dart:async';
import 'dart:convert' show utf8;

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show SessionUsage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:desktop_drop/desktop_drop.dart';
import 'package:material_ui/material_ui.dart';

import '../acp/client.dart' show AcpSessionInfo;
import '../desktop/control_client.dart' show hostWords;
import '../highlight/highlighted.dart' show highlightRenderers;
import '../history/session_import.dart' show earlierPage;
import '../l10n/app_text.dart';
import '../popout/hub_tabs.dart' show HubTabStrip;
import '../popout/popout_hub.dart' show PopoutHub;
import '../profile/report.dart';
import '../rail/session_groups.dart' show rowStateOf;
import '../selectable.dart';
import '../session_store.dart';
import '../shell/app_palette.dart';
import '../shell/layout_controls.dart';
import '../shell/layout_state.dart';
import '../shell/surface.dart';
import '../shell/work_panel.dart';
import '../surfaces/files_surface.dart';
import '../surfaces/subagents_surface.dart' show SubagentsSurface;
import '../surfaces/terminal_controller.dart' show TerminalWorkspace;
import '../surfaces/terminal_surface.dart';
import '../theme/look.dart';
import '../usage/spend_model.dart' show SpendRow;
import '../workspace/open_in.dart' show workspaceOpener;
import '../workspace/open_in_ui.dart';
import 'child_thread.dart';
import 'activity_strip.dart';
import 'composer.dart';
import 'delegation_row.dart';
import 'conversation_find.dart';
import 'conversation_tools.dart';
import 'delegation_view.dart';
import 'file_links.dart';
import 'inspector.dart' show changeTotals;
import 'inspector_surfaces.dart' show DetailsSurface;
import 'new_session_view.dart';
import 'presence.dart';
import 'reply_voice.dart';
import 'run_in_terminal.dart';
import 'session_views.dart';
import 'session_watch.dart' show SessionWatches;

/// The conversation view: header, transcript, permission/question/secret prompts, composer, and optional work panels.
class ConversationView extends StatefulWidget {
  const ConversationView({
    super.key,
    required this.store,
    required this.narrow,
    required this.handle,
    required this.layout,
    required this.surfaces,
    this.kit = const ComposerKit(),
    this.renderers = chat.ChatRenderers.none,
    required this.welcome,
    this.noAgent = false,
    this.sessionSpend,
    this.sessionWatches,
    this.chatOnly = false,
    this.onOpenFullApp,
  });

  final bool chatOnly;
  final VoidCallback? onOpenFullApp;
  final Widget welcome;
  final bool noAgent;
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;

  /// The watches of the host's shared sessions; null where this key may not arm one.
  final SessionWatches? sessionWatches;
  final ConversationHandle handle;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;
  final SessionStore store;
  final bool narrow;
  final ShellLayout layout;
  final WorkSurfaces surfaces;

  @override
  State<ConversationView> createState() => _ConversationViewState();
}

class _ConversationViewState extends State<ConversationView> {
  final TextEditingController _input = TextEditingController();
  final FocusNode _composerFocus = FocusNode();
  final _transcript = GlobalKey<chat.ChatTranscriptState>();

  /// The find bar of this conversation.
  final _find = ConversationFinder();

  void _onFind() {
    if (mounted) setState(() {});
  }

  void _keepText() {
    final session = widget.store.selected;
    if (session != null && session.isDraft) widget.store.keepDraftText(session, _input.text);
  }

  @override
  void initState() {
    super.initState();
    final session = widget.store.selected;
    if (session != null && (session.isDraft || session.draftText.isNotEmpty)) _input.text = session.draftText;
    _input.addListener(_keepText);
    widget.store.addListener(_followReplies);
    _find.addListener(_onFind);
    widget.handle
      ..owner = this
      ..focusComposer = _composerFocus.requestFocus
      ..showChanges = widget.chatOnly
          ? null
          : () {
              final session = widget.store.selected;
              if (session != null && !session.isDraft) _showInspector(session);
            }
      ..find = () {
        final session = widget.store.selected;
        if (session != null && !session.isDraft) _find.show();
      }
      ..tools = () {
        final session = widget.store.selected;
        return session == null || session.isDraft ? const [] : conversationTools(context, session, _find);
      }
      ..insert = (text) {
        _input.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
        _composerFocus.requestFocus();
      };
  }

  void _followReplies() {
    final reader = widget.kit.reader, session = widget.store.selected;
    if (reader == null || !reader.voiceMode || session == null || session.isDraft || session.openChild != null) return;
    followReplies(reader, session.id, session.messages, working: session.status == SessionStatus.working);
  }

  chat.ChatRenderers _renderersFor(ChatSession session) {
    final reader = widget.kit.reader;
    final openFile = _filesSurface(session) == null ? null : (String path) => _openInFiles(session, path);
    var renderers = highlightRenderers.merge(filePathRenderers(root: FilesSurface.workspaceOf(session), onOpenFile: openFile, reader: reader))
        .merge(delegationRenderers)
        .merge(terminalRenderers(offered: () => _terminalSurface(session) != null, run: (command) => _runInTerminal(session, command)));
    if (reader != null) renderers = renderers.merge(replyRenderers(reader));
    return _find.decorate(renderers.merge(widget.renderers), session);
  }

  void _addToComposer(String text) {
    final had = _input.text;
    final joined = had.isEmpty || had.endsWith('\n') ? '$had$text' : '$had\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
  }

  void _toggleSurface(ChatSession session, String id) {
    final surface = widget.surfaces.registry.where((each) => each.id == id).firstOrNull;
    if (surface == null) return;
    final mine = widget.surfaces.of(session.id);
    final open = mine.find(id);
    final layout = widget.layout;
    final slot = open?.slot ?? surface.placements.first;
    final panelOpen = slot == SurfaceSlot.right ? layout.rightOpen : layout.bottomOpen;
    if (open != null && panelOpen && layout.focus != PanelFocus.main && identical(mine.activeIn(slot), open)) {
      slot == SurfaceSlot.right ? layout.setRightOpen(false) : layout.setBottomOpen(false);
      return;
    }
    widget.store.refreshGit(session);
    mine.openSurface(surface);
    slot == SurfaceSlot.right ? layout.showRight() : layout.showBottom();
  }

  /// Opens [id] and brings its panel into view without closing it if already active.
  void _openSurface(ChatSession session, String id) {
    final surface = widget.surfaces.registry.where((each) => each.id == id && each.available(session)).firstOrNull;
    if (surface == null) return;
    widget.store.refreshGit(session);
    final opened = widget.surfaces.of(session.id).openSurface(surface);
    opened.slot == SurfaceSlot.bottom ? widget.layout.showBottom() : widget.layout.showRight();
  }

  /// The shells open for [session], if its Terminal surface has made any.
  TerminalWorkspace? _openTerminalWorkspace(ChatSession? session) {
    if (session == null || widget.chatOnly) return null;
    final open = widget.surfaces.of(session.id).find(TerminalSurface.surfaceId);
    return open == null ? null : TerminalSurface.openWorkspace(open.controller);
  }

  TerminalSurface? _terminalSurface(ChatSession session) {
    if (widget.chatOnly) return null;
    final surface = widget.surfaces.registry.whereType<TerminalSurface>().firstOrNull;
    return surface != null && surface.accessFor(session) is TerminalOnHost ? surface : null;
  }

  void _runInTerminal(ChatSession session, String command) {
    final surface = _terminalSurface(session);
    if (surface == null) return;
    final opened = widget.surfaces.of(session.id).openSurface(surface);
    opened.slot == SurfaceSlot.bottom ? widget.layout.showBottom() : widget.layout.showRight();
    surface.workspaceFor(opened.controller, session)?.focused?.type(command);
  }

  FilesSurface? _filesSurface(ChatSession session) {
    if (widget.chatOnly) return null;
    final surface = widget.surfaces.registry.whereType<FilesSurface>().firstOrNull;
    return surface != null && surface.available(session) ? surface : null;
  }

  void _openInFiles(ChatSession session, String path) {
    final surface = _filesSurface(session);
    if (surface == null) return;
    final opened = widget.surfaces.of(session.id).openSurface(surface, null, (controller) => controller.data[FilesSurface.revealKey] = FileReveal(path));
    opened.slot == SurfaceSlot.bottom ? widget.layout.showBottom() : widget.layout.showRight();
  }

  void _attachText(ChatSession session, String name, String text) {
    final messenger = ScaffoldMessenger.of(context);
    final problem = widget.store.addAttachment(session, name, utf8.encode(text), mimeType: 'text/plain');
    if (problem != null) messenger.showSnackBar(SnackBar(content: Text(problem)));
  }

  void _showInspector(ChatSession session) => _toggleSurface(session, DetailsSurface.surfaceId);

  @override
  void dispose() {
    if (identical(widget.handle.owner, this)) {
      widget.handle
        ..owner = null
        ..focusComposer = null
        ..showChanges = null
        ..find = null
        ..tools = null
        ..insert = null;
    }
    _find
      ..removeListener(_onFind)
      ..dispose();
    _input.removeListener(_keepText);
    widget.store.removeListener(_followReplies);
    _composerFocus.dispose();
    _input.dispose();
    super.dispose();
  }

  Future<void> _showEarlier(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final handle = widget.handle;
    final brought = <String>{};
    final picked = await showSelectableDialog<AcpSessionInfo>(
      context: context,
      builder: (context) => EarlierConversations(
        list: (cursor) => earlierPage(
          cursor,
          imported: (handle.canListImported?.call(session) ?? false) ? () => handle.imported!(session) : null,
          agent: widget.store.canListEarlier(session) ? (next) => widget.store.earlierSessions(session, cursor: next) : null,
          onImported: brought.addAll,
        ),
      ),
    );
    if (picked == null) return;
    try {
      if (brought.contains(picked.id)) {
        await handle.showImported!(session, picked);
      } else {
        await widget.store.openEarlier(session, picked);
      }
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That conversation could not be opened: $error')));
    }
  }

  void _queuedToComposer(String text) {
    final current = _input.text;
    final joined = current.trim().isEmpty ? text : '$current\n\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
    _composerFocus.requestFocus();
  }

  void _send({bool steer = false}) {
    final text = _input.text;
    final session = widget.store.selected;
    final attached = session?.attachments.isNotEmpty ?? false;
    if (text.trim().isEmpty && !attached) return;
    final starting = session != null && session.isDraft;
    try {
      _input.clear();
      unawaited(widget.store.sendText(text, steer: steer).catchError((Object error) {
        if (!mounted) return;
        if (session != null && !identical(widget.store.selected, session)) {
          widget.store.keepDraftText(session, text);
        } else if (_input.text.isEmpty) {
          _input.text = text;
        }
        final started = session?.remoteSessionId != null;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${steer ? 'Not steered' : starting && !started ? 'Not started' : 'Message not accepted'}: ${reasonOf(error)}')),
        );
      }));
    } on StateError {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text('Connect to a host before sending.')));
    }
    _transcript.currentState?.jumpToLatest();
  }

  static String _subtitle(ChatSession? session) => session == null ? 'Multi-session agent chat' : session.agent;

  static String _state(ChatSession? session) => session == null ? 'Offline' : rowStateOf(session).word;

  Future<void> _popOut(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final text = AppText.of(context);
    try {
      await widget.handle.popout!.popOut(session);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text(text('window.failed', {'why': hostWords(error)}))));
    }
  }

  (String, IconData, VoidCallback)? _workspaceEntry(ChatSession? session) {
    if (session == null) return null;
    final workspace = workspaceOpener.workspaceOf(place: session.place, cwd: session.cwd, endpoint: session.endpoint, peerId: session.peerId);
    if (workspace.target == null && workspace.reason == null) return null;
    return (
      'Open the workspace',
      Icons.folder_open,
      () => unawaited(showWorkspaceActions(context, opener: workspaceOpener, target: workspace.target, reason: workspace.reason)),
    );
  }

  List<Widget> _headerActions(ChatSession? session, {bool tight = false}) {
    final actions = <(String, IconData, VoidCallback)>[
      if (session != null && (widget.store.canListEarlier(session) || (widget.handle.canListImported?.call(session) ?? false)))
        ('Earlier conversations', Icons.history, () => unawaited(_showEarlier(session))),
      if (session != null && !session.isDraft) ...conversationTools(context, session, _find),
      if (session != null && !session.isDraft && !widget.chatOnly) ('Session details', Icons.view_sidebar_outlined, () => _showInspector(session)),
      if (session != null && !widget.chatOnly && (widget.handle.moveTargets?.call(session).isNotEmpty ?? false))
        ('Move to…', Icons.drive_file_move_outlined, () => unawaited(widget.handle.move!(context, session))),
      if (session != null && (!widget.narrow || Uri.base.queryParameters['view'] == 'panel') && !session.isDraft && (widget.handle.popout?.available ?? false))
        if (widget.handle.popout!.isPoppedOut(session))
          (context.t('window.bring_back'), Icons.close_fullscreen, () => unawaited(widget.handle.popout!.dockBack(session)))
        else
          (context.t('window.open'), Icons.open_in_new, () => unawaited(_popOut(session))),
      if (!widget.chatOnly) ?_workspaceEntry(session),
      if (widget.chatOnly && widget.onOpenFullApp != null) ('Open the full app', Icons.open_in_new, widget.onOpenFullApp!),
      if (session?.profile case final report?)
        ('Profile', Icons.tune, () => unawaited(showSelectableDialog<void>(context: context, builder: (_) => ProfileReportDialog(report: report)))),
    ];
    if (!(widget.narrow || tight) || actions.length < 2) {
      return [for (final (tooltip, icon, onPressed) in actions) IconButton(tooltip: tooltip, icon: Icon(icon), onPressed: onPressed)];
    }
    return [
      PopupMenuButton<VoidCallback>(
        tooltip: 'More',
        onSelected: (chosen) => chosen(),
        itemBuilder: (context) => [
          for (final (label, icon, onPressed) in actions) PopupMenuItem(value: onPressed, child: ListTile(leading: Icon(icon), title: Text(label))),
        ],
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final chosen = widget.store.selected;
    final session = chosen != null && widget.noAgent && chosen.isDraft && !widget.store.isStarting(chosen) ? null : chosen;
    if (session?.firstSent case final sent? when _input.text == sent && !widget.store.isStarting(session!)) {
      session.firstSent = null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _input.text == sent) _input.clear();
      });
    }

    final tabs = widget.chatOnly ? null : widget.handle.popout;
    // The start screen of a new session is no conversation yet: no title, state or close button over it, and no tab.
    final start = session != null && session.isDraft && !widget.store.isStarting(session);
    final conversation = Column(children: [
      if (tabs != null) HubTabStrip(hub: tabs, windowKey: PopoutHub.mainKey, narrow: widget.narrow),
      Material(
        color: context.look.bar,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: LayoutBuilder(
            builder: (context, headerBox) => Row(children: [
              if (widget.narrow)
                Builder(builder: (inner) => IconButton(tooltip: 'Sessions', icon: const Icon(Icons.menu), onPressed: () => Scaffold.of(inner).openDrawer())),
              Expanded(
                child: start ? const SizedBox.shrink() : Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(session?.title ?? 'Canoa', style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600), maxLines: 1, overflow: TextOverflow.ellipsis),
                  Text(_subtitle(session), style: Theme.of(context).textTheme.bodySmall, maxLines: 1, overflow: TextOverflow.ellipsis),
                ]),
              ),
              if (!start && session != null && !session.isDraft && !widget.chatOnly)
                if (changeTotals(session) case (final added, final removed))
                  _ChangesChip(added: added, removed: removed, onPressed: () => _showInspector(session)),
              if (!start && session != null && session.others.isNotEmpty) PresenceMark(others: session.others, narrow: widget.narrow),
              if (!start && session != null && widget.store.inBypass(session)) _BypassMark(narrow: widget.narrow),
              if (!start) ..._headerActions(session, tight: headerBox.maxWidth < 680),
              if (session != null && !start)
                IconButton(
                  tooltip: 'Close conversation',
                  icon: const Icon(Icons.close),
                  onPressed: () => unawaited((widget.handle.close ?? widget.store.close)(session)),
                ),
              if (session == null || start)
                const SizedBox.shrink()
              else if (widget.narrow || headerBox.maxWidth < 680)
                Tooltip(message: _state(session), child: const Padding(padding: EdgeInsets.all(8), child: Icon(Icons.hub_outlined, size: 18)))
              else
                Chip(key: const Key('session-state-chip'), avatar: const Icon(Icons.hub_outlined, size: 16), label: Text(_state(session)), visualDensity: VisualDensity.compact),
              if (!widget.narrow && !widget.chatOnly)
                LayoutControls(
                  layout: widget.layout,
                  section: ShellLayout.conversations,
                  hasIndex: true,
                  hasRight: session != null && widget.surfaces.offers(session, SurfaceSlot.right),
                  hasBottom: session != null && widget.surfaces.offers(session, SurfaceSlot.bottom),
                ),
            ]),
          ),
        ),
      ),
      if (session == null)
        Expanded(child: widget.welcome)
      else if (session.isDraft && !widget.store.isStarting(session))
        Expanded(child: _dropping(session, NewSessionView(store: widget.store, session: session, kit: widget.kit, composer: _composerFor(session, centred: true))))
      else if (session.openChild != null && session.tasks[session.openChild!] != null)
        Expanded(
          child: ChildThreadView(
            store: widget.store,
            session: session,
            task: session.tasks[session.openChild!]!,
            renderers: widget.kit.reader == null
                ? delegationRenderers.merge(widget.renderers)
                : delegationRenderers.merge(replyRenderers(widget.kit.reader!)).merge(widget.renderers),
            reader: widget.kit.reader,
            narrow: widget.narrow,
          ),
        )
      else ...[
        _find.bar(session),
        Expanded(
          child: _dropping(
            session,
            // The transcript is its own selection area: the one around the screen does not take part in its drag or Select all.
            SelectionContainer.disabled(
              child: chat.ChatTranscript(
                key: _transcript,
                entries: delegationEntries(session, onOpen: (id) => widget.store.openChild(session, id), now: widget.store.clock),
                terminals: session.terminals,
                plan: session.plan,
                working: session.status == SessionStatus.working || widget.store.isStarting(session),
                agentName: session.agent,
                density: _find.searching ? chat.ChatDensity.verbose : null,
                turns: [for (final turn in session.chat.turns) chat.TurnSpan.of(turn)],
                renderers: _renderersFor(session),
                narrow: widget.narrow,
              ),
            ),
          ),
        ),
        ListenableBuilder(
          listenable: widget.surfaces.of(session.id),
          builder: (context, _) {
            final workspace = _openTerminalWorkspace(session);
            Widget strip() {
              final openShells = workspace?.shells ?? 0;
              final runningShells = workspace?.runningShells ?? 0;
              if (!SessionActivityStrip.visibleFor(session, openShells: openShells, runningShells: runningShells)) {
                return const SizedBox.shrink();
              }
              return SessionActivityStrip(
                session: session,
                openShells: openShells,
                runningShells: runningShells,
                onOpenSubagents: widget.chatOnly ? null : () => _openSurface(session, SubagentsSurface.surfaceId),
                onOpenDetails: widget.chatOnly ? null : () => _openSurface(session, DetailsSurface.surfaceId),
                onOpenTerminal: widget.chatOnly ? null : () => _openSurface(session, TerminalSurface.surfaceId),
                onStopSubagents: () => unawaited(widget.store.stopTasks(session)),
              );
            }
            return workspace == null ? strip() : ListenableBuilder(listenable: workspace, builder: (context, _) => strip());
          },
        ),
        if (session.status == SessionStatus.working && session.tasks.live.isNotEmpty)
          DelegationWaiting(
            children: [for (final task in session.tasks.live) ChildView.of(task)],
            onStop: () => unawaited(widget.store.stopTasks(session)),
          ),
        if (session.accountOffer != null) SessionAccountOffer(store: widget.store, session: session),
        if (session.queue.isNotEmpty) SessionQueuePanel(store: widget.store, session: session, toComposer: _queuedToComposer),
        if (session.pendingPermissions.isNotEmpty) SessionPermissionCard(store: widget.store, session: session),
        if (session.pendingQuestions.isNotEmpty) SessionQuestionCard(store: widget.store, session: session),
        if (session.pendingSecrets.isNotEmpty) SessionSecretCard(store: widget.store, session: session),
        chat.UsageFooter(text: chat.describeUsage(SessionUsage(cost: session.usage.cost, currency: session.usage.currency), spend: session.spend)),
        Container(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
          child: Center(
            heightFactor: 1,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.chatLook.messageWidth + 100),
              child: Column(mainAxisSize: MainAxisSize.min, children: [
                _composerFor(session),
              ]),
            ),
          ),
        ),
      ],
    ]);
    if (session == null || widget.chatOnly) return conversation;
    return PanelFrame(
      layout: widget.layout,
      session: session,
      surfaces: widget.surfaces.of(session.id),
      registry: widget.surfaces,
      narrow: widget.narrow,
      child: conversation,
      wrap: (panels) => ListenableBuilder(
        listenable: widget.surfaces.of(session.id),
        builder: (context, _) {
          final workspace = _openTerminalWorkspace(session);
          Widget env() => SurfaceEnvironment(
            store: widget.store,
            sessionSpend: widget.sessionSpend,
            sessionWatches: widget.sessionWatches,
            onComment: _addToComposer,
            onAttachText: widget.store.canAttach(session) ? (name, text) => _attachText(session, name, text) : null,
            onOpenInFiles: _filesSurface(session) == null ? null : (path) => _openInFiles(session, path),
            onOpenSurface: (id) => _openSurface(session, id),
            windows: widget.handle.windows,
            openShells: workspace?.shells ?? 0,
            runningShells: workspace?.runningShells ?? 0,
            onOpenChild: () {
              if (widget.narrow) widget.layout.setRightOpen(false);
            },
            child: panels,
          );
          return workspace == null ? env() : ListenableBuilder(listenable: workspace, builder: (context, _) => env());
        },
      ),
    );
  }

  Widget _composerFor(ChatSession session, {bool centred = false}) => SessionComposer(
    store: widget.store,
    session: session,
    controller: _input,
    focusNode: _composerFocus,
    kit: widget.kit,
    centred: centred,
    enabled: !widget.store.isStarting(session) &&
        (session.status == SessionStatus.preview ||
            session.status == SessionStatus.draft ||
            session.status == SessionStatus.connected ||
            session.status == SessionStatus.working),
    onSend: _send,
    onSteer: () => _send(steer: true),
    working: session.status == SessionStatus.working,
    onStop: () {
      final messenger = ScaffoldMessenger.of(context);
      unawaited(widget.store.cancelActiveSession().catchError((Object error) {
        messenger.showSnackBar(SnackBar(content: Text('Could not stop: $error')));
      }));
    },
    commands: session.commands,
  );

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

class _ChangesChip extends StatelessWidget {
  const _ChangesChip({required this.added, required this.removed, required this.onPressed});

  final int added, removed;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final mono = TextStyle(fontFamily: look.codeFont, fontSize: 12, fontWeight: FontWeight.w600);
    return Tooltip(
      message: 'Changes',
      child: InkWell(
        key: const ValueKey('changes chip'),
        onTap: onPressed,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            Text('+$added', style: mono.copyWith(color: look.success)),
            const SizedBox(width: 4),
            Text('−$removed', style: mono.copyWith(color: look.danger)),
          ]),
        ),
      ),
    );
  }
}

class _BypassMark extends StatelessWidget {
  const _BypassMark({required this.narrow});

  final bool narrow;

  static const message = 'Bypass: each request that offers to allow is allowed without asking you.';

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: message,
      child: Padding(
        key: const Key('bypass-mark'),
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: narrow
            ? Icon(Icons.fast_forward_outlined, size: 18, color: scheme.tertiary)
            : Chip(
                avatar: Icon(Icons.fast_forward_outlined, size: 16, color: scheme.onTertiaryContainer),
                label: Text('Bypass', style: TextStyle(color: scheme.onTertiaryContainer)),
                backgroundColor: scheme.tertiaryContainer,
                side: BorderSide.none,
                visualDensity: VisualDensity.compact,
              ),
      ),
    );
  }
}
