import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:xterm2/xterm.dart';

import '../session_store.dart';
import '../shell/surface.dart';
import '../theme/look.dart';
import '../shell/panel_tabs.dart' show PanelShows, PanelTabKind;
import '../shell/shown.dart';
import 'files_surface.dart' show FilesSurface;
import 'terminal_clipboard.dart';
import 'terminal_controller.dart';
import 'terminal_keys.dart';

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
  const TerminalSurface(this.accessFor, {this.kinds, this.clipboard = const SystemTerminalClipboard()});

  static const surfaceId = 'terminal';

  /// The kinds of terminal tab the host lists (an extension's names a shell of its own); the host's own is the surface itself.
  final List<PanelTabKind> Function()? kinds;

  /// The clipboard the shells copy to and paste from.
  final TerminalClipboard clipboard;

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

  /// The shells of [controller]'s surface for [session], made with a first one (running [firstShell], the host's own choice when
  /// null) the first time; null where the host offers none.
  TerminalWorkspace? workspaceFor(SurfaceController controller, ChatSession session, {String? firstShell}) {
    if (controller.data[_key] case final TerminalWorkspace made) return made;
    if (accessFor(session) case TerminalOnHost(:final host)) {
      final made = TerminalWorkspace(host: host, cwd: FilesSurface.workspaceOf(session), clipboard: clipboard)..newTab(shell: firstShell);
      controller.data[_key] = made;
      controller.onRelease(made.release);
      return made;
    }
    return null;
  }

  /// The shells are the panel's own tabs, so the panel has one tab bar and the shells have none of their own.
  @override
  List<SurfaceTab> tabs(ChatSession session, SurfaceController controller) {
    final workspace = workspaceFor(controller, session);
    if (workspace == null) return const [];
    return [
      for (final tab in workspace.tabs)
        SurfaceTab(
          id: '${tab.id}',
          title: tab.name,
          selected: identical(tab, workspace.active),
          select: () => workspace.activate(tab),
          close: () => workspace.closeTab(tab),
        ),
    ];
  }

  @override
  Listenable? tabsChanged(ChatSession session, SurfaceController controller) => workspaceFor(controller, session);

  @override
  List<SurfaceAction> actions(BuildContext context, ChatSession session, SurfaceController controller) {
    final workspace = workspaceFor(controller, session);
    final tab = workspace?.active;
    if (workspace == null || tab == null) return const [];
    final pane = workspace.focused;
    final attach = SurfaceEnvironment.maybeOf(context)?.onAttachText;
    return [
      SurfaceAction(id: 'split', label: 'Split: a second shell beside this one', enabled: tab.panes.length < 2 && !workspace.full, run: (_) => workspace.split(tab)),
      SurfaceAction(id: 'rename', label: 'Rename the tab', run: (context) => unawaited(renameTerminalTab(context, workspace, tab))),
      if (pane != null) ...[
        SurfaceAction(id: 'copy', label: 'Copy the selected text', enabled: pane.selection != null, run: (context) => unawaited(copyFromTerminal(context, pane))),
        SurfaceAction(id: 'paste', label: 'Paste into the shell', run: (context) => unawaited(pasteIntoTerminal(context, pane))),
      ],
      if (attach != null && pane != null) ...[
        SurfaceAction(
          id: 'attach selection',
          label: 'Attach the selected text to your message',
          enabled: pane.selectedText != null,
          run: (_) => attach('terminal selection.txt', pane.selectedText!),
        ),
        SurfaceAction(
          id: 'attach output',
          label: 'Attach the last output to your message',
          run: (_) {
            final text = pane.lastOutput();
            if (text != null) attach('terminal output.txt', text);
          },
        ),
      ],
    ];
  }

  /// The new-shell entries: the host's own choice, then each shell the host lists, then the tab kinds an extension names a shell for.
  /// They are offered whether or not a terminal is open, since each opens a new shell.
  @override
  Future<List<NewTab>> newTabs(ChatSession session, SessionSurfaces surfaces) async {
    if (accessFor(session) is! TerminalOnHost) return const [];
    final TerminalOnHost(:host) = accessFor(session)! as TerminalOnHost;
    final open = surfaces.find(id);
    final workspace = open == null ? null : workspaceFor(open.controller, session);
    final full = workspace?.full ?? false;
    NewTab entry(String key, String label, IconData icon, String? shell) => NewTab(
      key: key,
      label: label,
      icon: icon,
      enabled: !full,
      open: (slot) {
        final existing = surfaces.find(id);
        if (existing == null) {
          surfaces.openSurface(this, slot, (controller) => workspaceFor(controller, session, firstShell: shell));
        } else {
          surfaces.openSurface(this, slot);
          workspaceFor(existing.controller, session)?.newTab(shell: shell);
        }
      },
    );
    final listed = [for (final each in kinds?.call() ?? const <PanelTabKind>[]) if (each.shows == PanelShows.terminal) each];
    final own = listed.where((each) => each.id == surfaceId).firstOrNull;
    final shells = await host.shells();
    final label = own?.title ?? title;
    return [
      entry('open $id', label, own?.iconData ?? icon, own?.shell),
      for (final shell in shells) entry('open $id shell ${shell.id}', '$label: ${shell.label}', own?.iconData ?? icon, shell.id),
      for (final each in listed)
        if (each.id != surfaceId) entry('open $id kind ${each.id}', each.title, each.iconData, each.shell),
    ];
  }

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    if (accessFor(session) case TerminalRefused(:final reason)) {
      return Center(child: Padding(padding: const EdgeInsets.all(16), child: Text(reason, key: const ValueKey('terminal refused'), textAlign: TextAlign.center)));
    }
    final workspace = workspaceFor(controller, session);
    if (workspace == null) return const Center(child: Text('This host offers no terminal.'));
    return TerminalPanel(workspace: workspace, showToolbar: false);
  }
}

/// The terminal's text: the bundled monospace font first, so a cell is as wide as every glyph drawn in it on any platform, then the
/// faces xterm lists for what it lacks (CJK, emoji, symbols).
const terminalTextStyle = TerminalStyle(
  fontSize: 13,
  fontFamily: 'Source Code Pro',
  fontFamilyFallback: [
    'Menlo',
    'Monaco',
    'Consolas',
    'Liberation Mono',
    'Courier New',
    'Noto Sans Mono CJK SC',
    'Noto Sans Mono CJK TC',
    'Noto Sans Mono CJK KR',
    'Noto Sans Mono CJK JP',
    'Noto Sans Mono CJK HK',
    'Noto Color Emoji',
    'Noto Sans Symbols',
    'monospace',
  ],
);

/// The terminal's tabs and screens. The shell the person chose (a new one, a split, another tab), and the panel when it is brought
/// in front, take the keyboard.
class TerminalPanel extends StatefulWidget {
  const TerminalPanel({super.key, required this.workspace, this.onAttachText, this.showToolbar = true, this.focusOnMount = false});

  final TerminalWorkspace workspace;
  final void Function(String name, String text)? onAttachText;

  /// Whether the panel draws its own bar of shells. A conversation's panel has one tab bar, which holds the shells, so it asks for
  /// none; the Terminal section has no other, so it keeps this one.
  final bool showToolbar;

  /// Whether the shell takes the keyboard when the panel is first drawn: true where drawing it is the person's choice (the Terminal
  /// section), false where it is also drawn when they move to another conversation.
  final bool focusOnMount;

  @override
  State<TerminalPanel> createState() => _TerminalPanelState();
}

class _TerminalPanelState extends State<TerminalPanel> {
  bool? _wasShown;

  TerminalWorkspace get workspace => widget.workspace;

  @override
  void initState() {
    super.initState();
    if (widget.focusOnMount) _takeKeyboard();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final shown = Shown.of(context);
    if (shown && _wasShown == false) _takeKeyboard();
    _wasShown = shown;
  }

  /// Gives the keyboard to the focused shell once the frame is built, which is when a screen just chosen is drawn and can have it.
  void _takeKeyboard() => WidgetsBinding.instance.addPostFrameCallback((_) {
    if (mounted) workspace.focused?.focusNode.requestFocus();
  });

  Future<void> _rename(BuildContext context, TerminalTab tab) => renameTerminalTab(context, workspace, tab);

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
          if (pane != null) ...[
            ListenableBuilder(
              listenable: pane.view,
              builder: (context, _) => IconButton(
                key: const ValueKey('terminal copy'),
                tooltip: 'Copy the selected text',
                visualDensity: VisualDensity.compact,
                iconSize: 18,
                icon: const Icon(Icons.content_copy_outlined),
                onPressed: pane.selection == null ? null : () => unawaited(copyFromTerminal(context, pane)),
              ),
            ),
            IconButton(
              key: const ValueKey('terminal paste'),
              tooltip: 'Paste into the shell',
              visualDensity: VisualDensity.compact,
              iconSize: 18,
              icon: const Icon(Icons.content_paste_outlined),
              onPressed: () => unawaited(pasteIntoTerminal(context, pane)),
            ),
          ],
          if (widget.onAttachText != null && pane != null) ...[
            ListenableBuilder(
              listenable: pane.view,
              builder: (context, _) => IconButton(
                key: const ValueKey('terminal attach selection'),
                tooltip: 'Attach the selected text to your message',
                visualDensity: VisualDensity.compact,
                iconSize: 18,
                icon: const Icon(Icons.content_cut),
                onPressed: pane.selectedText == null ? null : () => widget.onAttachText!('terminal selection.txt', pane.selectedText!),
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
                if (text != null) widget.onAttachText!('terminal output.txt', text);
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
      if (workspace.takeFocusWish()) _takeKeyboard();
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (widget.showToolbar) ...[_toolbar(context), const Divider(height: 1)],
        Expanded(
          child: active == null
              ? Center(child: FilledButton.icon(key: const ValueKey('terminal start'), onPressed: workspace.newTab, icon: const Icon(Icons.add), label: const Text('New shell')))
              : Stack(fit: StackFit.expand, children: [
                  // Every tab stays built, so its screen and scroll position are where they were left.
                  for (final tab in workspace.tabs)
                    Shown(
                      key: ValueKey('terminal screen ${tab.id}'),
                      shown: identical(tab, active),
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

/// Asks for a new name for [tab] and renames it.
Future<void> renameTerminalTab(BuildContext context, TerminalWorkspace workspace, TerminalTab tab) async {
  final name = await showDialog<String>(context: context, builder: (context) => _RenameDialog(name: tab.name));
  if (name != null) workspace.rename(tab, name);
}

/// Says [text] where the person looks, when there is somewhere to say it.
void _tell(BuildContext context, String text) => ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(text)));

/// Copies what is selected in [pane]; says why when the platform refused.
Future<void> copyFromTerminal(BuildContext context, TerminalPane pane) async {
  try {
    await pane.copy();
  } on ClipboardRefused catch (refused) {
    if (context.mounted) _tell(context, refused.message);
  }
}

/// Pastes the clipboard into [pane] as the menu's Paste does; says why when the platform refused or the clipboard holds no text.
Future<void> pasteIntoTerminal(BuildContext context, TerminalPane pane) async {
  final problem = await pane.pasteClipboard();
  if (problem != null && context.mounted) _tell(context, problem);
}

class _Screen extends StatefulWidget {
  const _Screen({super.key, required this.pane, required this.onFocus, required this.onClose});

  final TerminalPane pane;
  final VoidCallback onFocus;
  final VoidCallback onClose;

  @override
  State<_Screen> createState() => _ScreenState();
}

class _ScreenState extends State<_Screen> {
  static const _theme = TerminalThemes.defaultTheme;

  TerminalPane get pane => widget.pane;
  late final VoidCallback _stopListening;

  @override
  void initState() {
    super.initState();
    // A paste the platform delivers itself (a browser's paste event) goes to the shell that has the keyboard, and only to it.
    _stopListening = pane.clipboard.onPaste((text) {
      if (!pane.focusNode.hasPrimaryFocus) return false;
      pane.paste(text);
      pane.view.clearSelection();
      return true;
    });
  }

  @override
  void dispose() {
    _stopListening();
    super.dispose();
  }

  Future<void> _menu(BuildContext context, Offset at) async {
    final overlay = Overlay.of(context).context.findRenderObject()! as RenderBox;
    final choice = await showMenu<String>(
      context: context,
      position: RelativeRect.fromRect(at & Size.zero, Offset.zero & overlay.size),
      items: [
        PopupMenuItem(key: const ValueKey('terminal context copy'), value: 'copy', enabled: pane.selection != null, child: const Text('Copy')),
        const PopupMenuItem(key: ValueKey('terminal context paste'), value: 'paste', child: Text('Paste')),
        const PopupMenuItem(key: ValueKey('terminal context select all'), value: 'all', child: Text('Select all')),
      ],
    );
    if (!context.mounted) return;
    switch (choice) {
      case 'copy':
        await copyFromTerminal(context, pane);
      case 'paste':
        await pasteIntoTerminal(context, pane);
      case 'all':
        pane.selectAll();
    }
  }

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
              onPointerDown: (_) => widget.onFocus(),
              child: TerminalView(
                pane.terminal,
                key: ValueKey('terminal view ${pane.hashCode}'),
                controller: pane.view,
                focusNode: pane.focusNode,
                padding: const EdgeInsets.all(6),
                theme: _theme,
                textStyle: terminalTextStyle,
                // The clipboard keys are `terminalClipboardKey`'s; the terminal's own shortcuts, which use the system clipboard
                // directly and take Ctrl+A from the shell, are left out.
                shortcuts: const {},
                onKeyEvent: (_, event) => terminalClipboardKey(event, pane, say: (problem) {
                  if (mounted) _tell(this.context, problem);
                }),
                onSecondaryTapDown: (details, _) => unawaited(_menu(context, details.globalPosition)),
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
                  TextButton(key: const ValueKey('terminal close shell'), onPressed: widget.onClose, child: const Text('Close')),
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
