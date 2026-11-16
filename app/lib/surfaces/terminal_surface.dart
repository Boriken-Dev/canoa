import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:xterm/xterm.dart';

import '../session_store.dart';
import '../shell/surface.dart';
import '../theme/look.dart';
import 'files_surface.dart' show FilesSurface;
import 'terminal_controller.dart';

/// What a conversation's host offers of a shell.
sealed class TerminalAccess {
  const TerminalAccess();
}

/// The host grants this app a terminal: shells open on [host].
final class TerminalOnHost extends TerminalAccess {
  const TerminalOnHost(this.host);

  final TerminalHost host;
}

/// The conversation's agent is reached somewhere that offers no shell; the surface says [reason] and opens none.
final class TerminalRefused extends TerminalAccess {
  const TerminalRefused(this.reason);

  final String reason;
}

/// An interactive shell on the host the session runs on, in the session's workspace, in tabs with a second shell beside any of
/// them. Hiding it or moving it to the other panel leaves the shells running; closing the surface ends them. Nothing a shell prints
/// reaches the agent: the person attaches a selection or the last output to their own message.
class TerminalSurface extends WorkSurface {
  const TerminalSurface(this.accessFor);

  static const surfaceId = 'terminal';

  /// What the host [session] runs on offers; null when it offers nothing to this app and the surface is not listed.
  final TerminalAccess? Function(ChatSession session) accessFor;

  @override
  String get id => surfaceId;
  @override
  String get title => 'Terminal';
  @override
  IconData get icon => Icons.terminal;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.bottom, SurfaceSlot.right];

  @override
  bool available(ChatSession session) => accessFor(session) != null;

  static const _key = 'terminal workspace';

  /// The shells of [controller]'s surface for [session], made with a first one the first time; null where the host offers none.
  TerminalWorkspace? workspaceFor(SurfaceController controller, ChatSession session) {
    if (controller.data[_key] case final TerminalWorkspace made) return made;
    if (accessFor(session) case TerminalOnHost(:final host)) {
      final made = TerminalWorkspace(host: host, cwd: FilesSurface.workspaceOf(session))..newTab();
      controller.data[_key] = made;
      controller.onRelease(made.release);
      return made;
    }
    return null;
  }

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    if (accessFor(session) case TerminalRefused(:final reason)) {
      return Center(child: Padding(padding: const EdgeInsets.all(16), child: Text(reason, key: const ValueKey('terminal refused'), textAlign: TextAlign.center)));
    }
    final workspace = workspaceFor(controller, session);
    if (workspace == null) return const Center(child: Text('This host offers no terminal.'));
    return TerminalPanel(workspace: workspace, onAttachText: SurfaceEnvironment.maybeOf(context)?.onAttachText);
  }
}

/// The terminal's tabs and screens.
class TerminalPanel extends StatelessWidget {
  const TerminalPanel({super.key, required this.workspace, this.onAttachText});

  final TerminalWorkspace workspace;
  final void Function(String name, String text)? onAttachText;

  Future<void> _rename(BuildContext context, TerminalTab tab) async {
    final name = await showDialog<String>(context: context, builder: (context) => _RenameDialog(name: tab.name));
    if (name != null) workspace.rename(tab, name);
  }

  Widget _toolbar(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final active = workspace.active;
    final pane = workspace.focused;
    return Material(
      color: context.look.bar,
      child: SizedBox(
        height: 38,
        child: Row(children: [
          Expanded(
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
              children: [
                for (final tab in workspace.tabs)
                  Padding(
                    padding: const EdgeInsets.only(right: 4),
                    child: Semantics(
                      button: true,
                      selected: identical(tab, active),
                      label: tab.name,
                      child: Material(
                        color: identical(tab, active) ? context.look.selected : Colors.transparent,
                        borderRadius: BorderRadius.circular(6),
                        child: InkWell(
                          key: ValueKey('terminal tab ${tab.id}'),
                          borderRadius: BorderRadius.circular(6),
                          onTap: () => workspace.activate(tab),
                          child: Padding(
                            padding: const EdgeInsets.only(left: 10),
                            child: Row(mainAxisSize: MainAxisSize.min, children: [
                              Text(tab.name, style: TextStyle(fontSize: 12.5, fontWeight: identical(tab, active) ? FontWeight.w600 : FontWeight.w400)),
                              if (tab.panes.length > 1) Padding(padding: const EdgeInsets.only(left: 4), child: Icon(Icons.vertical_split_outlined, size: 13, color: scheme.onSurfaceVariant)),
                              IconButton(
                                key: ValueKey('terminal close tab ${tab.id}'),
                                tooltip: 'Close ${tab.name}',
                                visualDensity: VisualDensity.compact,
                                iconSize: 13,
                                icon: const Icon(Icons.close),
                                onPressed: () => workspace.closeTab(tab),
                              ),
                            ]),
                          ),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
          IconButton(
            key: const ValueKey('terminal new'),
            tooltip: workspace.full ? 'The host holds ${TerminalWorkspace.limit} shells at most' : 'New shell',
            visualDensity: VisualDensity.compact,
            iconSize: 18,
            icon: const Icon(Icons.add),
            onPressed: workspace.full ? null : workspace.newTab,
          ),
          IconButton(
            key: const ValueKey('terminal split'),
            tooltip: 'Split: a second shell beside this one',
            visualDensity: VisualDensity.compact,
            iconSize: 18,
            icon: const Icon(Icons.vertical_split_outlined),
            onPressed: active == null || active.panes.length >= 2 || workspace.full ? null : () => workspace.split(active),
          ),
          if (active != null)
            PopupMenuButton<String>(
              key: const ValueKey('terminal menu'),
              tooltip: 'Tab actions',
              iconSize: 18,
              padding: EdgeInsets.zero,
              onSelected: (_) => unawaited(_rename(context, active)),
              itemBuilder: (_) => const [PopupMenuItem(key: ValueKey('terminal rename'), value: 'rename', child: Text('Rename the tab'))],
            ),
          if (onAttachText != null && pane != null) ...[
            ListenableBuilder(
              listenable: pane.view,
              builder: (context, _) => IconButton(
                key: const ValueKey('terminal attach selection'),
                tooltip: 'Attach the selected text to your message',
                visualDensity: VisualDensity.compact,
                iconSize: 18,
                icon: const Icon(Icons.content_cut),
                onPressed: pane.selectedText == null ? null : () => onAttachText!('terminal selection.txt', pane.selectedText!),
              ),
            ),
            IconButton(
              key: const ValueKey('terminal attach output'),
              tooltip: 'Attach the last output to your message',
              visualDensity: VisualDensity.compact,
              iconSize: 18,
              icon: const Icon(Icons.attach_file),
              onPressed: () {
                final text = pane.lastOutput();
                if (text != null) onAttachText!('terminal output.txt', text);
              },
            ),
          ],
          const SizedBox(width: 4),
        ]),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: workspace,
    builder: (context, _) {
      final active = workspace.active;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        _toolbar(context),
        const Divider(height: 1),
        Expanded(
          child: active == null
              ? Center(child: FilledButton.icon(key: const ValueKey('terminal start'), onPressed: workspace.newTab, icon: const Icon(Icons.add), label: const Text('New shell')))
              : Stack(fit: StackFit.expand, children: [
                  // Every tab stays built, so its screen and scroll position are where they were left.
                  for (final tab in workspace.tabs)
                    Offstage(
                      key: ValueKey('terminal screen ${tab.id}'),
                      offstage: !identical(tab, active),
                      child: Row(children: [
                        for (final (index, pane) in tab.panes.indexed) ...[
                          if (index > 0) VerticalDivider(width: 1, color: Theme.of(context).dividerColor),
                          Expanded(child: _Screen(key: ObjectKey(pane), pane: pane, onFocus: () => workspace.focus(tab, pane), onClose: () => workspace.closePane(tab, pane))),
                        ],
                      ]),
                    ),
                ]),
        ),
      ]);
    },
  );
}

class _Screen extends StatelessWidget {
  const _Screen({super.key, required this.pane, required this.onFocus, required this.onClose});

  final TerminalPane pane;
  final VoidCallback onFocus;
  final VoidCallback onClose;

  static const _theme = TerminalThemes.defaultTheme;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: pane,
    builder: (context, _) {
      final look = context.chatLook;
      final note = switch (pane.state) {
        PaneState.starting => 'Starting the shell…',
        PaneState.running => null,
        PaneState.exited => 'The shell ended (exit status ${pane.exitCode}).',
        PaneState.failed => pane.problem ?? 'The shell could not be started.',
      };
      return ColoredBox(
        color: _theme.background,
        child: Column(children: [
          Expanded(
            child: Listener(
              onPointerDown: (_) => onFocus(),
              child: TerminalView(
                pane.terminal,
                key: ValueKey('terminal view ${pane.hashCode}'),
                controller: pane.view,
                autofocus: true,
                padding: const EdgeInsets.all(6),
                theme: _theme,
                textStyle: const TerminalStyle(fontSize: 13),
              ),
            ),
          ),
          if (note != null)
            Container(
              key: const ValueKey('terminal note'),
              color: const Color(0xFF2A2A2A),
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              child: Row(children: [
                Expanded(child: Text(note, style: TextStyle(fontSize: 12, color: pane.state == PaneState.failed ? look.danger : const Color(0xFFCCCCCC)))),
                if (pane.state == PaneState.exited || pane.state == PaneState.failed) ...[
                  TextButton(key: const ValueKey('terminal restart'), onPressed: pane.start, child: const Text('Start again')),
                  TextButton(key: const ValueKey('terminal close shell'), onPressed: onClose, child: const Text('Close')),
                ],
              ]),
            ),
        ]),
      );
    },
  );
}

class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.name});

  final String name;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final TextEditingController _field = TextEditingController(text: widget.name);

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Rename the tab'),
    content: TextField(key: const ValueKey('terminal rename field'), controller: _field, autofocus: true, onSubmitted: (value) => Navigator.pop(context, value)),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(key: const ValueKey('terminal rename ok'), onPressed: () => Navigator.pop(context, _field.text), child: const Text('Rename')),
    ],
  );
}
