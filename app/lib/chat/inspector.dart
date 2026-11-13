import 'package:coqui_acp_chat/coqui_acp_chat.dart' show SessionUsage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../acp/pipe_channel.dart' show isPipeUrl;
import '../session_store.dart';
import '../sources/source_icons.dart' show SourceMark;
import '../theme/look.dart';
import 'delegation_row.dart' show Ticking, stateColor;
import 'delegation_view.dart' show ChildView, lineage;
import 'git_changes.dart';
import 'git_changes_pane.dart';
import 'modes.dart';
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
      return (label: 'Host', value: name ?? session.place.label);
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
  const SessionInspectorPanel({super.key, required this.store, required this.session, required this.onComment, required this.onClose, this.onOpenChild});

  final SessionStore store;
  final ChatSession session;

  /// Adds text to the message being written.
  final ValueChanged<String> onComment;
  final VoidCallback onClose;

  /// Called after a child's thread is opened from the lineage, for a sheet that has to get out of its way.
  final VoidCallback? onOpenChild;

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
          onBack: () => setState(() => _diff = null),
          onComment: widget.onComment,
        );
      }
      return SessionInspector(
        store: widget.store,
        session: widget.session,
        scopes: scopes,
        scope: scope,
        onScope: (chosen) => setState(() => _scope = chosen),
        onOpenDiff: () => setState(() => _diff = scope),
        onClose: widget.onClose,
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
  });

  final SessionStore store;
  final ChatSession session;
  final List<ChangeScope> scopes;
  final ChangeScope scope;
  final ValueChanged<ChangeScope> onScope;
  final VoidCallback onOpenDiff;
  final VoidCallback onClose;

  /// Opens the thread of the child with this id.
  final ValueChanged<String>? onOpenChild;

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
    final running = [for (final entry in session.terminals.entries) if (entry.value.isRunning) (entry.key, entry.value)];
    final live = children.where((child) => child.live).length;
    return Material(
      color: context.look.rail,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 8, 0),
          child: Row(children: [
            Expanded(child: Text('Session', style: Theme.of(context).textTheme.titleMedium)),
            IconButton(tooltip: 'Close details', onPressed: onClose, icon: const Icon(Icons.close)),
          ]),
        ),
        const Divider(height: 1),
        Expanded(
          child: ListView(padding: const EdgeInsets.fromLTRB(16, 12, 16, 16), children: [
            Text('Environment', style: heading),
            const SizedBox(height: 6),
            _Fact(machine.label, machine.value),
            if (folder != null && folder.isNotEmpty) _Fact('Folder', folder),
            if (branch != null) _Fact('Branch', branch),
            if (worktree != null) _Fact('Worktree', worktree.path),
            if (mode != null) _Fact('Mode', mode),
            const SizedBox(height: 16),
            Text('Context', style: heading),
            const SizedBox(height: 6),
            _ContextMeter(usage: session.usage),
            if (children.isNotEmpty) ...[
              const SizedBox(height: 16),
              Text(live > 0 ? 'Lineage · $live running' : 'Lineage', style: heading),
              const SizedBox(height: 4),
              _Lineage(children: children, now: store.clock, onOpen: onOpenChild),
            ],
            if (running.isNotEmpty) ...[
              const SizedBox(height: 16),
              Text('Running commands · ${running.length}', style: heading),
              const SizedBox(height: 4),
              for (final (id, terminal) in running) _CommandRow(key: ValueKey('command $id'), command: terminal.command ?? id),
            ],
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

/// One row for each child the session handed out: its source's mark, title, state and how long it has run. A row opens the child's
/// thread.
class _Lineage extends StatelessWidget {
  const _Lineage({required this.children, required this.now, required this.onOpen});

  final List<ChildView> children;
  final DateTime Function() now;
  final ValueChanged<String>? onOpen;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Ticking(
      active: children.any((child) => child.live),
      builder: (context) => Column(key: const ValueKey('lineage panel'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (final child in children)
          InkWell(
            key: ValueKey('lineage ${child.id}'),
            borderRadius: BorderRadius.circular(look.radius / 2),
            onTap: onOpen == null ? null : () => onOpen!(child.id),
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: Row(children: [
                SizedBox.square(dimension: 18, child: Center(child: SourceMark.provider(child.provider, size: 14))),
                const SizedBox(width: 8),
                Expanded(child: Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 13))),
                const SizedBox(width: 8),
                Text(child.word, style: TextStyle(color: stateColor(look, child), fontSize: 12)),
                if (child.elapsedAt(now()) case final elapsed?)
                  Padding(padding: const EdgeInsets.only(left: 8), child: Text(spanLabel(elapsed), style: TextStyle(color: look.muted, fontSize: 12))),
              ]),
            ),
          ),
      ]),
    );
  }
}

/// A command the agent started that has not reported its end.
class _CommandRow extends StatelessWidget {
  const _CommandRow({super.key, required this.command});

  final String command;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(children: [
        Icon(Icons.terminal, size: 16, color: look.muted),
        const SizedBox(width: 8),
        Expanded(child: Text(command, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(fontFamily: look.codeFont, fontSize: 12))),
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

class _ContextMeter extends StatelessWidget {
  const _ContextMeter({required this.usage});

  final SessionUsage usage;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final percent = usage.percent;
    if (percent == null) return Text('Not reported yet.', key: const ValueKey('context none'), style: TextStyle(color: look.muted, fontSize: 13));
    final used = usage.used!, size = usage.size!;
    return Column(key: const ValueKey('context meter'), crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('$percent% of context', style: const TextStyle(fontSize: 13)),
      const SizedBox(height: 6),
      ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: LinearProgressIndicator(
          value: percent / 100,
          minHeight: 6,
          color: percent >= 90 ? look.danger : (percent >= 75 ? look.warning : look.accent),
          backgroundColor: look.border,
        ),
      ),
      const SizedBox(height: 4),
      Text('${compactCount(used)} of ${compactCount(size)} tokens', style: TextStyle(color: look.muted, fontSize: 12)),
    ]);
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
  final VoidCallback onBack;
  final ValueChanged<String> onComment;

  @override
  Widget build(BuildContext context) => Material(
    color: context.look.rail,
    child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
        child: Row(children: [
          IconButton(tooltip: 'Back to the session', onPressed: onBack, icon: const Icon(Icons.arrow_back)),
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
    ]),
  );
}
