import 'dart:async' show unawaited;

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show SessionUsage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../acp/pipe_channel.dart' show isPipeUrl;
import '../session_store.dart';
import '../theme/look.dart';
import '../shell/surface.dart' show SurfaceEnvironment;
import '../usage/spend_model.dart' show SpendRow;
import '../usage/tokens_view.dart' show tokenColumns;
import '../workspace/open_in.dart' show WorkspaceOpener;
import 'background_tasks.dart';
import 'context_kinds.dart';
import 'context_pill.dart' show compactPrompt;
import 'delegation_view.dart' show lineage;
import 'git_changes.dart';
import 'git_changes_pane.dart';
import 'git_publish.dart';
import 'modes.dart';
import 'outputs.dart';
import 'outputs_view.dart';
import 'session_views.dart';

/// The `+N −M` a session's changes add up to: git's, when its folder is read, else what the agent reported.
(int, int)? changeTotals(ChatSession session) {
  final git = session.git.whole;
  if (git != null) return git.files.isEmpty && git.added == 0 && git.removed == 0 ? null : (git.added, git.removed);
  final reported = sessionChanges(session);
  return reported.isEmpty ? null : totalOf(reported);
}

/// Where the machine the agent runs on is named: this computer, or the host a hosted session was reached at.
({String label, String value}) machineOf(ChatSession session) {
  switch (session.place) {
    case SessionPlace.local:
      return (label: 'Machine', value: 'This computer');
    case SessionPlace.host || SessionPlace.peer:
      final endpoint = session.endpoint;
      final name = endpoint == null
          ? null
          : isPipeUrl(endpoint.toString())
              ? 'this computer, through its canoa'
              : (endpoint.host.isEmpty ? null : endpoint.host);
      // The host a draft was aimed at keeps the name it was saved under; one reached any other way is named by its address.
      return (label: 'Host', value: session.host ?? name ?? session.place.label);
    case SessionPlace.browser:
      return (label: 'Host', value: session.place.label);
  }
}

/// The mode a session is in, as the mode menu names it; null before the agent says.
String? modeLabelOf(SessionStore store, ChatSession session) {
  final value = store.modeOf(session);
  if (value == null) return null;
  return modeEntries(modeOptionIn(session.options)).where((entry) => entry.value == value).firstOrNull?.label ?? value;
}

/// The inspector's two states: the session's facts, or the diff one of its changes views opens.
class SessionInspectorPanel extends StatefulWidget {
  const SessionInspectorPanel({
    super.key,
    required this.store,
    required this.session,
    required this.onComment,
    required this.onClose,
    this.onOpenChild,
    this.onOpenInFiles,
    this.openShells = 0,
    this.runningShells = 0,
    this.onOpenTerminal,
    this.onOpenSubagents,
    this.opener,
    this.embedded = false,
    this.diffOnly = false,
  });

  final SessionStore store;
  final ChatSession session;

  /// Adds text to the message being written.
  final ValueChanged<String> onComment;
  final VoidCallback onClose;

  /// Called after a child's thread is opened from the lineage, for a sheet that has to get out of its way.
  final VoidCallback? onOpenChild;

  /// Opens a path in the Files surface; null where it has nothing to show for this session.
  final ValueChanged<String>? onOpenInFiles;

  /// Open and running interactive shells for [session].
  final int openShells, runningShells;

  /// Opens the interactive terminal surface when shells exist.
  final VoidCallback? onOpenTerminal;

  /// Opens the Subagents surface when subagents exist.
  final VoidCallback? onOpenSubagents;

  /// What can be done with a path on this computer; null when the session's files are on another host.
  final WorkspaceOpener? opener;

  /// Whether the panel sits under a tab of the work panel, which has the title and the close button.
  final bool embedded;

  /// Whether it shows the diff alone, with no way back to the session's facts: the Changes surface.
  final bool diffOnly;

  @override
  State<SessionInspectorPanel> createState() => _SessionInspectorPanelState();
}

class _SessionInspectorPanelState extends State<SessionInspectorPanel> {
  ChangeScope? _diff;
  ChangeScope _scope = ChangeScope.reported;

  @override
  void initState() {
    super.initState();
    // What the disk holds is read when the inspector opens, so it is not older than the person's last look.
    widget.store.refreshGit(widget.session);
    if (widget.diffOnly) _diff = _scope;
  }

  /// The views the session can show: the agent's report where it made one, git's where the folder is a repository it reads.
  List<ChangeScope> _scopes() => [
    if (sessionChanges(widget.session).isNotEmpty || !widget.session.git.known) ChangeScope.reported,
    if (widget.session.git.known) ...[ChangeScope.uncommitted, ChangeScope.lastTurn, ChangeScope.wholeSession],
  ];

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.store,
    builder: (context, _) {
      final scopes = _scopes();
      final scope = scopes.contains(_scope) ? _scope : scopes.first;
      if (_diff != null) {
        return _DiffView(
          store: widget.store,
          session: widget.session,
          scope: scope,
          scopes: scopes,
          onScope: (chosen) => setState(() => _diff = _scope = chosen),
          onBack: widget.diffOnly ? null : () => setState(() => _diff = null),
          onComment: widget.onComment,
        );
      }
      return SessionInspector(
        store: widget.store,
        session: widget.session,
        scopes: scopes,
        scope: scope,
        sessionSpend: SurfaceEnvironment.maybeOf(context)?.sessionSpend,
        onScope: (chosen) => setState(() => _scope = chosen),
        onOpenDiff: () => setState(() => _diff = scope),
        onClose: widget.onClose,
        embedded: widget.embedded,
        onOpenInFiles: widget.onOpenInFiles,
        openShells: widget.openShells,
        runningShells: widget.runningShells,
        onOpenTerminal: widget.onOpenTerminal,
        onOpenSubagents: widget.onOpenSubagents,
        opener: widget.opener,
        onOpenChild: (id) {
          widget.store.openChild(widget.session, id);
          widget.onOpenChild?.call();
        },
      );
    },
  );
}

/// What a session is, in one place: where it runs, in which folder, branch and mode, how full its context is, and what it has
/// changed.
class SessionInspector extends StatelessWidget {
  const SessionInspector({
    super.key,
    required this.store,
    required this.session,
    required this.scopes,
    required this.scope,
    required this.onScope,
    required this.onOpenDiff,
    required this.onClose,
    this.onOpenChild,
    this.onOpenInFiles,
    this.openShells = 0,
    this.runningShells = 0,
    this.onOpenTerminal,
    this.onOpenSubagents,
    this.opener,
    this.sessionSpend,
    this.embedded = false,
  });

  final int openShells, runningShells;
  final VoidCallback? onOpenTerminal, onOpenSubagents;

  final bool embedded;

  /// What the host recorded this session spending; null where the host keeps no usage for this person.
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;
  final SessionStore store;
  final ChatSession session;
  final List<ChangeScope> scopes;
  final ChangeScope scope;
  final ValueChanged<ChangeScope> onScope;
  final VoidCallback onOpenDiff;
  final VoidCallback onClose;

  /// Opens the thread of the child with this id.
  final ValueChanged<String>? onOpenChild;

  /// Opens a path in the Files surface; null where it has nothing to show for this session.
  final ValueChanged<String>? onOpenInFiles;

  /// What can be done with a path on this computer; null when the session's files are on another host.
  final WorkspaceOpener? opener;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final machine = machineOf(session);
    final worktree = session.worktree;
    final folder = worktree?.repo ?? session.cwd;
    final branch = session.git.whole?.branch ?? worktree?.branch;
    final mode = modeLabelOf(store, session);
    final heading = Theme.of(context).textTheme.titleSmall?.copyWith(fontWeight: FontWeight.w600);
    final children = lineage(session);
    final commands = commandsOf(session);
    final running = BackgroundTasks.runningIn(children, commands) + runningShells;
    final hasBackground = children.isNotEmpty || commands.isNotEmpty || openShells > 0;
    final stoppableChildren = children.any((child) => child.live && !child.native);
    // The folder the Files surface shows, so a row's path reads the same in both.
    final workspace = worktree?.path ?? session.cwd;
    final outputs = outputsOf(session, workspace);
    Widget backgroundSection() => Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SizedBox(height: 16),
      Row(children: [
        Expanded(child: Text(running > 0 ? 'Background tasks · $running running' : 'Background tasks', style: heading)),
        if (stoppableChildren)
          TextButton(
            key: const ValueKey('inspector stop all tasks'),
            style: TextButton.styleFrom(visualDensity: VisualDensity.compact, padding: const EdgeInsets.symmetric(horizontal: 8), minimumSize: Size.zero),
            onPressed: () => unawaited(store.stopTasks(session)),
            child: const Text('Stop all'),
          ),
        if (children.isNotEmpty && onOpenSubagents != null)
          TextButton(
            key: const ValueKey('inspector open subagents'),
            style: TextButton.styleFrom(visualDensity: VisualDensity.compact, padding: const EdgeInsets.symmetric(horizontal: 8), minimumSize: Size.zero),
            onPressed: onOpenSubagents,
            child: const Text('Subagents'),
          ),
      ]),
      const SizedBox(height: 4),
      if (openShells > 0)
        InkWell(
          key: const ValueKey('inspector open shells'),
          borderRadius: BorderRadius.circular(look.radius / 2),
          onTap: onOpenTerminal,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Row(children: [
              SizedBox.square(dimension: 18, child: Icon(Icons.terminal, size: 16, color: runningShells > 0 ? look.accent : look.muted)),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  runningShells > 0
                      ? (runningShells == 1 ? '1 interactive shell running' : '$runningShells interactive shells running')
                      : (openShells == 1 ? '1 interactive shell open' : '$openShells interactive shells open'),
                  style: const TextStyle(fontSize: 13),
                ),
              ),
              if (onOpenTerminal != null) Icon(Icons.chevron_right, size: 16, color: look.muted),
            ]),
          ),
        ),
      if (children.isNotEmpty || commands.isNotEmpty)
        BackgroundTasks(
          children: children,
          commands: commands,
          now: store.clock,
          onOpenChild: onOpenChild,
          onStopChild: (id) => unawaited(store.stopTask(session, id)),
        ),
    ]);
    Widget outputsSection() => Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SizedBox(height: 16),
      Text('Outputs · ${outputs.length}', style: heading),
      const SizedBox(height: 4),
      OutputsList(outputs: outputs, root: workspace, onOpenInFiles: onOpenInFiles, opener: opener),
    ]);
    return Material(
      color: context.look.rail,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (!embedded) ...[
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 8, 0),
            child: Row(children: [
              Expanded(child: Text('Session', style: Theme.of(context).textTheme.titleMedium)),
              IconButton(tooltip: 'Close details', onPressed: onClose, icon: const Icon(Icons.close)),
            ]),
          ),
          const Divider(height: 1),
        ],
        Expanded(
          child: ListView(padding: const EdgeInsets.fromLTRB(16, 12, 16, 16), children: [
            Text('Environment', style: heading),
            const SizedBox(height: 6),
            _Fact(machine.label, machine.value),
            if (session.kind == SessionKind.chat)
              const _Fact('Kind', 'Chat')
            else if (folder != null && folder.isNotEmpty)
              _Fact('Workspace', folder),
            if (branch != null) _Fact('Branch', branch),
            if (worktree != null) _Fact('Worktree', worktree.path),
            if (mode != null) _Fact('Mode', mode),
            if (session.kind == SessionKind.chat && folder != null && folder.isNotEmpty)
              Theme(
                data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
                child: ExpansionTile(
                  key: const Key('inspector-technical'),
                  tilePadding: EdgeInsets.zero,
                  childrenPadding: EdgeInsets.zero,
                  dense: true,
                  title: Text('Technical details', style: TextStyle(color: look.muted, fontSize: 12)),
                  children: [_Fact('Path', folder)],
                ),
              ),
            const SizedBox(height: 16),
            Text('Context', style: heading),
            const SizedBox(height: 6),
            _ContextMeter(
              usage: session.usage,
              kinds: session.contextKinds,
              onCompact: identical(store.selected, session) && compactPrompt(session) != null
                  ? () => unawaited(store.sendText(compactPrompt(session)!))
                  : null,
            ),
            if (sessionSpend != null && session.remoteSessionId != null) ...[
              const SizedBox(height: 16),
              Text('Tokens spent', style: heading),
              const SizedBox(height: 6),
              _SessionTokens(sessionId: session.remoteSessionId!, load: sessionSpend!, revision: session.usage.used),
            ],
            if (hasBackground && running > 0) backgroundSection(),
            if (outputs.isNotEmpty) outputsSection(),
            if (hasBackground && running == 0) backgroundSection(),
            const SizedBox(height: 16),
            Text('Changes', style: heading),
            const SizedBox(height: 6),
            _ChangesSummary(session: session, scopes: scopes, scope: scope, onScope: onScope, onOpen: onOpenDiff, look: look),
          ]),
        ),
      ]),
    );
  }
}

class _Fact extends StatelessWidget {
  const _Fact(this.label, this.value);

  final String label, value;

  @override
  Widget build(BuildContext context) {
    final muted = TextStyle(color: context.chatLook.muted, fontSize: 12);
    return Padding(
      key: ValueKey('fact $label'),
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        SizedBox(width: 76, child: Text(label, style: muted)),
        Expanded(child: Text(value, style: const TextStyle(fontSize: 13), maxLines: 3, overflow: TextOverflow.ellipsis)),
      ]),
    );
  }
}

/// How full the context window is: what is used out of what there is, a bar split by the kinds the agent names, and a row for
/// each. An agent that names none gets the two rows the figures give, used and free, and a line saying the rest is not reported.
class _ContextMeter extends StatelessWidget {
  const _ContextMeter({required this.usage, required this.kinds, this.onCompact});

  final SessionUsage usage;
  final List<ContextKind> kinds;
  final VoidCallback? onCompact;

  static String _share(int tokens, int size) => '${(100 * tokens / size).toStringAsFixed(1)}%';

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final percent = usage.percent;
    if (percent == null) return Text('Not reported yet.', key: const ValueKey('context none'), style: TextStyle(color: look.muted, fontSize: 13));
    final used = usage.used!, size = usage.size!;
    final named = kinds.where((kind) => !isFreeSpace(kind) && !kind.deferred).toList();
    final deferred = kinds.where((kind) => kind.deferred).toList();
    final reportedFree = kinds.where(isFreeSpace).firstOrNull?.tokens;
    final free = reportedFree ?? (size - used).clamp(0, size);
    final palette = [look.accent, look.warning, look.success, look.danger, look.muted];
    final muted = TextStyle(color: look.muted, fontSize: 12);
    Widget row(String key, Color color, String label, int tokens, String share) => Padding(
      key: ValueKey(key),
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(children: [
        Container(width: 10, height: 10, decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(2))),
        const SizedBox(width: 8),
        Expanded(child: Text(label, style: const TextStyle(fontSize: 13), overflow: TextOverflow.ellipsis)),
        Text(compactCount(tokens), style: muted),
        SizedBox(width: 52, child: Text(share, textAlign: TextAlign.end, style: muted)),
      ]),
    );
    final segments = named.isEmpty ? [(used, palette.first)] : [for (var i = 0; i < named.length; i++) (named[i].tokens, palette[i % palette.length])];
    return Column(key: const ValueKey('context meter'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Row(children: [
        Expanded(child: Text('$percent% used', style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600))),
        Text('${compactCount(used)} / ${compactCount(size)}', style: muted),
        if (onCompact != null) ...[
          const SizedBox(width: 8),
          TextButton(
            key: const ValueKey('context-inspector-compact'),
            style: TextButton.styleFrom(visualDensity: VisualDensity.compact, padding: const EdgeInsets.symmetric(horizontal: 8)),
            onPressed: onCompact,
            child: const Text('Compact'),
          ),
        ],
      ]),
      const SizedBox(height: 6),
      ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox(
          height: 6,
          child: ColoredBox(
            color: look.border,
            child: Row(key: const ValueKey('context bar'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final (tokens, color) in segments)
                if (tokens > 0) Expanded(flex: tokens, child: ColoredBox(color: color)),
              if (free > 0) Expanded(flex: free, child: const SizedBox.shrink()),
            ]),
          ),
        ),
      ),
      const SizedBox(height: 6),
      if (named.isEmpty)
        row('context used', palette.first, 'Used', used, _share(used, size))
      else
        for (var i = 0; i < named.length; i++) row('context kind ${named[i].label}', palette[i % palette.length], named[i].label, named[i].tokens, _share(named[i].tokens, size)),
      row('context free', look.border, 'Free space', free, _share(free, size)),
      for (final kind in deferred) row('context kind ${kind.label}', look.border, kind.label, kind.tokens, '—'),
      if (kinds.isEmpty)
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text('This agent does not say what its context holds.', key: const ValueKey('context unreported'), style: muted),
        ),
    ]);
  }
}

/// The tokens the host recorded this session spending: what it was billed for, which is not how full its context is.
class _SessionTokens extends StatefulWidget {
  const _SessionTokens({required this.sessionId, required this.load, required this.revision});

  final String sessionId;
  final Future<SpendRow?> Function(String sessionId) load;

  /// Changes when the agent reports again, which is when the figures are read again.
  final Object? revision;

  @override
  State<_SessionTokens> createState() => _SessionTokensState();
}

class _SessionTokensState extends State<_SessionTokens> {
  late Future<SpendRow?> _row = widget.load(widget.sessionId);

  @override
  void didUpdateWidget(_SessionTokens old) {
    super.didUpdateWidget(old);
    if (old.sessionId != widget.sessionId || old.revision != widget.revision) _row = widget.load(widget.sessionId);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final muted = TextStyle(color: look.muted, fontSize: 12);
    return FutureBuilder<SpendRow?>(
      future: _row,
      builder: (context, snapshot) {
        final row = snapshot.data;
        if (snapshot.connectionState != ConnectionState.done) return Text('Reading…', key: const ValueKey('tokens loading'), style: muted);
        if (row == null || row.records == 0) return Text('Nothing recorded yet.', key: const ValueKey('tokens none'), style: muted);
        return Column(
          key: const ValueKey('tokens spent'),
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final column in tokenColumns)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 2),
                child: Row(children: [
                  Expanded(child: Tooltip(message: column.help, child: Text(column.label, style: muted))),
                  Text(column.text(row), key: ValueKey('tokens ${column.label}'), style: TextStyle(fontSize: 13, color: column.of(row) == null ? look.muted : null)),
                ]),
              ),
          ],
        );
      },
    );
  }
}

class _ChangesSummary extends StatelessWidget {
  const _ChangesSummary({required this.session, required this.scopes, required this.scope, required this.onScope, required this.onOpen, required this.look});

  final ChatSession session;
  final List<ChangeScope> scopes;
  final ChangeScope scope;
  final ValueChanged<ChangeScope> onScope;
  final VoidCallback onOpen;
  final ChatLook look;

  @override
  Widget build(BuildContext context) {
    final mono = TextStyle(fontFamily: look.codeFont, fontSize: 12);
    final git = scope == ChangeScope.reported ? null : session.git;
    final files = git == null ? [for (final file in sessionChanges(session)) (file.path, file.added, file.removed)] : [for (final file in git.filesOf(scope)) (file.path, file.added, file.removed)];
    var added = 0, removed = 0;
    for (final (_, plus, minus) in files) {
      added += plus;
      removed += minus;
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Wrap(spacing: 6, runSpacing: 4, children: [
        for (final each in scopes)
          ChoiceChip(
            key: ValueKey('scope ${each.label}'),
            label: Text(each.label),
            selected: each == scope,
            visualDensity: VisualDensity.compact,
            onSelected: (_) => onScope(each),
          ),
      ]),
      const SizedBox(height: 6),
      Text(scope.note, style: TextStyle(color: look.muted, fontSize: 12)),
      const SizedBox(height: 8),
      if (files.isEmpty)
        Text(scope == ChangeScope.reported ? 'No file has changed yet.' : 'No file differs here.', key: const ValueKey('changes none'), style: const TextStyle(fontSize: 13))
      else ...[
        InkWell(
          key: const ValueKey('open diff'),
          onTap: onOpen,
          borderRadius: BorderRadius.circular(look.radius / 2),
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: Row(children: [
              Text('+$added', key: const ValueKey('changes added'), style: mono.copyWith(color: look.success, fontWeight: FontWeight.w600)),
              const SizedBox(width: 6),
              Text('−$removed', key: const ValueKey('changes removed'), style: mono.copyWith(color: look.danger, fontWeight: FontWeight.w600)),
              const SizedBox(width: 8),
              Expanded(child: Text('${files.length} ${files.length == 1 ? 'file' : 'files'}', style: const TextStyle(fontSize: 13))),
              const Text('Open diff', style: TextStyle(fontSize: 13)),
              const Icon(Icons.chevron_right, size: 18),
            ]),
          ),
        ),
        for (final (path, plus, minus) in files.take(6))
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 2),
            child: Row(children: [
              Expanded(child: Text(path, style: mono, overflow: TextOverflow.ellipsis)),
              Text('+$plus', style: mono.copyWith(color: look.success)),
              const SizedBox(width: 6),
              Text('−$minus', style: mono.copyWith(color: look.danger)),
            ]),
          ),
        if (files.length > 6) Padding(padding: const EdgeInsets.only(top: 2), child: Text('and ${files.length - 6} more', style: TextStyle(color: look.muted, fontSize: 12))),
      ],
    ]);
  }
}

/// A changes view opened from the inspector: the agent's report in the package's pane, git's in [GitChangesPane].
class _DiffView extends StatelessWidget {
  const _DiffView({
    required this.store,
    required this.session,
    required this.scope,
    required this.scopes,
    required this.onScope,
    required this.onBack,
    required this.onComment,
  });

  final SessionStore store;
  final ChatSession session;
  final ChangeScope scope;
  final List<ChangeScope> scopes;
  final ValueChanged<ChangeScope> onScope;
  final VoidCallback? onBack;
  final ValueChanged<String> onComment;

  /// Commit, Push and Pull request, where the host does them for the person.
  Widget? _publishBar() {
    final publisher = store.gitPublisherOf(session), folder = store.gitFolderOf(session);
    if (publisher == null || folder == null || !session.git.known) return null;
    final disk = session.git.uncommitted;
    return GitPublishBar(
      publisher: publisher,
      folder: folder,
      files: disk?.files ?? const [],
      more: disk?.more ?? false,
      canCommit: scope == ChangeScope.uncommitted,
      branch: session.git.whole?.branch,
      onChanged: () => store.refreshGit(session),
    );
  }

  @override
  Widget build(BuildContext context) => Material(
    color: context.look.rail,
    child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
        child: Row(children: [
          if (onBack != null) IconButton(tooltip: 'Back to the session', onPressed: onBack, icon: const Icon(Icons.arrow_back)),
          Expanded(
            child: Wrap(spacing: 6, runSpacing: 4, children: [
              for (final each in scopes)
                ChoiceChip(
                  key: ValueKey('diff scope ${each.label}'),
                  label: Text(each.label),
                  selected: each == scope,
                  visualDensity: VisualDensity.compact,
                  onSelected: (_) => onScope(each),
                ),
            ]),
          ),
        ]),
      ),
      Expanded(
        child: scope == ChangeScope.reported
            ? SessionChangesPane(store: store, session: session, onComment: onComment)
            : GitChangesPane(scope: scope, files: session.git.filesOf(scope), more: session.git.whole?.more ?? false, onComment: onComment),
      ),
      if (scope != ChangeScope.reported) ?_publishBar(),
    ]),
  );
}
