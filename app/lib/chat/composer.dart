import 'dart:async';

import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/foundation.dart' show defaultTargetPlatform, TargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../session_store.dart';
import '../sources/source_icons.dart';
import '../sources/sources.dart';
import '../theme/look.dart';
import 'agent_events.dart' show AgentCommandInfo, Choice, ConfigOption;
import 'attachments.dart';
import 'cache_lifetime.dart';
import 'modes.dart';
import 'session_options.dart';
import 'transcript.dart' show CommandSuggestions;
import 'usage.dart';

/// What was copied: an image, files by their paths, or text.
abstract interface class ClipboardReader {
  Future<Uint8List?> image();
  Future<List<String>> files();
  Future<String?> text();
}

/// Text only, which is all Flutter's own clipboard reads.
final class TextClipboard implements ClipboardReader {
  const TextClipboard();
  @override
  Future<Uint8List?> image() async => null;
  @override
  Future<List<String>> files() async => const [];
  @override
  Future<String?> text() async => (await Clipboard.getData(Clipboard.kTextPlain))?.text;
}

/// What the composer offers beyond the session itself, from the app: where an agent can come from, folders, files and the
/// clipboard, and the overlays a session can start with.
final class ComposerKit {
  const ComposerKit({
    this.sources = const [],
    this.checks = const {},
    this.recentFolders = const [],
    this.inGitRepository,
    this.pickFolder,
    this.scratchFolder,
    this.pickFiles,
    this.statFile,
    this.clipboard = const TextClipboard(),
    this.overlays = const [],
    this.pasteAsFile = defaultPasteAsFile,
    this.warnPermille = defaultWarnPermille,
    this.onSourceSettings,
    this.onProfile,
  });

  /// The most characters a paste puts in the text; a longer one goes as an attachment of its own.
  static const defaultPasteAsFile = 8000;

  /// The sources Settings has, in its order.
  final List<AgentSource> sources;

  /// What each source needs and whether it is here, once checked.
  final Map<AgentSource, SourceCheck> checks;

  /// Folders sessions worked in, most recent first.
  final List<String> recentFolders;
  final bool Function(String folder)? inGitRepository;
  final Future<String?> Function()? pickFolder;

  /// The folder a draft with none chosen works in; null where there is none.
  final String? Function(ChatSession session)? scratchFolder;
  final Future<List<XFile>> Function()? pickFiles;

  /// A file's size in bytes, or null where it cannot be read.
  final Future<int?> Function(String path)? statFile;
  final ClipboardReader clipboard;

  /// canoa.cfg's overlays, by name, in their order.
  final List<String> overlays;
  final int pasteAsFile;

  /// From this many thousandths used, the usage control shows a status.
  final int warnPermille;

  /// Opens Settings at its sources.
  final VoidCallback? onSourceSettings;

  /// Shows what a session was given of its profile.
  final void Function(ChatSession session)? onProfile;
}

/// Adds the files at [paths] to [session]: an image by its bytes, anything else by its path. Answers what went wrong, by file.
Future<List<String>> addPaths(SessionStore store, ChatSession session, List<String> paths, ComposerKit kit,
    {Future<List<int>> Function(String path)? read}) async {
  final problems = <String>[];
  for (final path in paths) {
    final name = path.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).lastOrNull ?? path;
    try {
      final size = await kit.statFile?.call(path);
      if (size == null) {
        problems.add('$name cannot be read.');
        continue;
      }
      final String? problem;
      if (imageTypeOf(name, null) != null && size <= maxAttachedImage) {
        problem = store.addAttachment(session, name, await (read ?? store.readFile)(path), path: path);
      } else {
        problem = store.addFile(session, name, path, size);
      }
      if (problem != null) problems.add(problem);
    } on Object catch (error) {
      problems.add('$name: $error');
    }
  }
  return problems;
}

/// The message being written, what goes with it, and a row of controls under it: `+`, the source and Local or Worktree
/// before the session starts, the agent's mode, model and effort, the overlays, and Send.
class Composer extends StatefulWidget {
  const Composer({
    super.key,
    required this.store,
    required this.session,
    required this.controller,
    required this.kit,
    required this.onSend,
    this.focusNode,
    this.enabled = true,
    this.working = false,
    this.onStop,
    this.commands = const [],
    this.centred = false,
  });

  final SessionStore store;
  final ChatSession session;
  final TextEditingController controller;
  final FocusNode? focusNode;
  final ComposerKit kit;
  final bool enabled;
  final VoidCallback onSend;
  final bool working;
  final VoidCallback? onStop;
  final List<AgentCommandInfo> commands;

  /// Whether it is the middle of a new session's empty view rather than docked under a conversation.
  final bool centred;

  @override
  State<Composer> createState() => ComposerState();
}

class ComposerState extends State<Composer> {
  void _say(String text) => ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(text)));

  void _sayAll(List<String> problems) {
    if (problems.isNotEmpty) _say(problems.join('\n'));
  }

  Future<void> _addFiles() async {
    final picked = await widget.kit.pickFiles?.call() ?? const <XFile>[];
    if (!mounted) return;
    final problems = <String>[];
    for (final file in picked) {
      if (file.path.isNotEmpty && widget.kit.statFile != null) {
        problems.addAll(await addPaths(widget.store, widget.session, [file.path], widget.kit));
      } else {
        final problem = widget.store.addAttachment(widget.session, file.name, await file.readAsBytes(), mimeType: file.mimeType);
        if (problem != null) problems.add(problem);
      }
    }
    if (mounted) _sayAll(problems);
  }

  Future<void> _addFolder() async {
    final folder = await widget.kit.pickFolder?.call();
    if (folder == null || !mounted) return;
    final name = folder.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).lastOrNull ?? folder;
    final problem = widget.store.addFolder(widget.session, name, folder);
    if (problem != null) _say(problem);
  }

  /// What the clipboard holds: an image becomes an image attachment, copied files file attachments, and text stays text
  /// unless it is longer than [ComposerKit.pasteAsFile] characters.
  Future<void> paste() async {
    final clipboard = widget.kit.clipboard;
    final image = await clipboard.image();
    if (!mounted) return;
    if (image != null && image.isNotEmpty) {
      var n = 1;
      while (widget.session.attachments.any((item) => item.name == 'Pasted image $n.png')) {
        n++;
      }
      final problem = widget.store.addAttachment(widget.session, 'Pasted image $n.png', image, mimeType: 'image/png');
      if (problem != null) _say(problem);
      return;
    }
    final files = await clipboard.files();
    if (!mounted) return;
    if (files.isNotEmpty) {
      _sayAll(await addPaths(widget.store, widget.session, files, widget.kit));
      return;
    }
    final text = await clipboard.text();
    if (text == null || text.isEmpty || !mounted) return;
    if (text.length > widget.kit.pasteAsFile) {
      final problem = widget.store.addText(widget.session, text);
      if (problem != null) _say(problem);
      return;
    }
    final value = widget.controller.value;
    final selection = value.selection.isValid ? value.selection : TextSelection.collapsed(offset: value.text.length);
    final joined = value.text.replaceRange(selection.start, selection.end, text);
    widget.controller.value = TextEditingValue(
      text: joined,
      selection: TextSelection.collapsed(offset: selection.start + text.length),
    );
  }

  /// Shift+Tab puts the session in its next mode; Ctrl+V, or ⌘V on a Mac, pastes through [paste]; every other key goes on as
  /// usual.
  KeyEventResult onKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent && event.logicalKey == LogicalKeyboardKey.tab && HardwareKeyboard.instance.isShiftPressed &&
        !HardwareKeyboard.instance.isControlPressed && !HardwareKeyboard.instance.isAltPressed &&
        widget.session.status != SessionStatus.preview) {
      unawaited(widget.store.cycleMode(widget.session));
      return KeyEventResult.handled;
    }
    if (event is! KeyDownEvent || event.logicalKey != LogicalKeyboardKey.keyV || !widget.enabled) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    final mac = defaultTargetPlatform == TargetPlatform.macOS || defaultTargetPlatform == TargetPlatform.iOS;
    if ((mac ? keys.isMetaPressed : keys.isControlPressed) && !keys.isAltPressed && !keys.isShiftPressed) {
      unawaited(paste());
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final session = widget.session;
    final store = widget.store;
    final field = TextField(
      controller: widget.controller,
      focusNode: widget.focusNode,
      enabled: widget.enabled,
      minLines: widget.centred ? 3 : 1,
      maxLines: widget.centred ? 10 : 5,
      textInputAction: TextInputAction.send,
      onSubmitted: (_) => widget.onSend(),
      decoration: InputDecoration(
        hintText: !widget.enabled
            ? (store.isStarting(session) ? 'Starting ${session.agent}…' : 'Connect an ACP peer to chat')
            : session.isDraft
            ? 'Message ${session.source?.displayName ?? 'an agent'}: it starts when you send'
            : 'Message agent',
        border: InputBorder.none,
        isDense: true,
        contentPadding: const EdgeInsets.fromLTRB(4, 8, 4, 8),
      ),
    );
    final box = Container(
      key: const Key('composer'),
      padding: const EdgeInsets.fromLTRB(10, 6, 6, 6),
      decoration: BoxDecoration(color: look.composer, borderRadius: BorderRadius.circular(look.radius + 2)),
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (session.attachments.isNotEmpty) AttachmentChips(store: store, session: session),
        field,
        _Controls(composer: this),
      ]),
    );
    return Column(mainAxisSize: MainAxisSize.min, children: [
      ValueListenableBuilder<TextEditingValue>(
        valueListenable: widget.controller,
        builder: (context, value, _) => CommandSuggestions(
          commands: widget.commands,
          typed: value.text,
          onPick: (command) {
            final text = '/${command.name} ';
            widget.controller.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
          },
        ),
      ),
      const SizedBox(height: 4),
      box,
    ]);
  }
}

/// What goes with the next message, each with what it is, its size, how it will reach the agent when that is not as it is,
/// and a way to take it back.
class AttachmentChips extends StatelessWidget {
  const AttachmentChips({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final error = Theme.of(context).colorScheme.error;
    return Padding(
      padding: const EdgeInsets.only(bottom: 4, top: 2),
      child: Wrap(spacing: 6, runSpacing: 6, children: [
        for (final attachment in session.attachments)
          Builder(builder: (context) {
            final refused = store.refusalOf(session, attachment);
            final note = refused ?? store.routeNote(session, attachment);
            final thumbnail = attachment.isImage && attachment.bytes != null
                ? ClipRRect(
                    borderRadius: BorderRadius.circular(4),
                    child: Image.memory(Uint8List.fromList(attachment.bytes!), width: 28, height: 28, fit: BoxFit.cover,
                        gaplessPlayback: true, errorBuilder: (_, _, _) => const Icon(Icons.image_outlined, size: 20)),
                  )
                : Icon(switch (attachment.kind) {
                    AttachmentKind.folder => Icons.folder_outlined,
                    AttachmentKind.text => Icons.notes,
                    _ => Icons.description_outlined,
                  }, size: 20, color: look.muted);
            final chip = Container(
              key: ValueKey('attachment ${attachment.name}'),
              constraints: const BoxConstraints(maxWidth: 260),
              padding: const EdgeInsets.fromLTRB(4, 3, 2, 3),
              decoration: BoxDecoration(
                border: Border.all(color: refused != null ? error : look.muted.withValues(alpha: 0.35)),
                borderRadius: BorderRadius.circular(8),
              ),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                thumbnail,
                const SizedBox(width: 6),
                Flexible(child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(attachment.name, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 12)),
                  Text(
                    note ?? (attachment.kind == AttachmentKind.folder ? 'Folder' : sizeLabel(attachment.size)),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(fontSize: 11, color: refused != null ? error : look.muted),
                  ),
                ])),
                IconButton(
                  tooltip: 'Remove ${attachment.name}',
                  visualDensity: VisualDensity.compact,
                  iconSize: 16,
                  onPressed: () => store.removeAttachment(session, attachment),
                  icon: const Icon(Icons.close),
                ),
              ]),
            );
            return note == null ? chip : Tooltip(message: note, child: chip);
          }),
      ]),
    );
  }
}

/// A compact control in the row under the message.
class _Pill extends StatelessWidget {
  const _Pill({required this.label, this.icon, this.danger = false, this.status = false, this.enabled = true, this.maxWidth = 180});

  final String label;
  final IconData? icon;

  /// [danger] is for what is wrong; [status] for what is on, in the accent of the scheme, never the colour of an error.
  final bool danger, status, enabled;

  /// The widest the label gets before it is cut short.
  final double maxWidth;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final scheme = Theme.of(context).colorScheme;
    final color = danger ? scheme.error : status ? scheme.tertiary : (enabled ? null : look.muted);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        if (icon != null) ...[Icon(icon, size: 16, color: color ?? look.muted), const SizedBox(width: 4)],
        Flexible(
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: maxWidth),
            child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 12.5, color: color)),
          ),
        ),
        if (enabled) Icon(Icons.expand_more, size: 16, color: color ?? look.muted),
      ]),
    );
  }
}

/// The row under the message.
class _Controls extends StatelessWidget {
  const _Controls({required this.composer});

  /// From this width the controls stay on one line.
  static const double wideFrom = 600;

  final ComposerState composer;

  @override
  Widget build(BuildContext context) {
    final widget = composer.widget;
    final store = widget.store;
    final session = widget.session;
    final kit = widget.kit;
    final draft = session.status == SessionStatus.draft;
    final options = store.optionsOf(session);
    final seen = store.optionsSeen(session);
    // The mode first at the bottom left, then Add; the model and effort sit beside Send, as Claude Code and Codex lay them out.
    final left = <Widget>[
      _ModeChooser(store: store, session: session, options: options, seen: seen),
      _UsagePill(session: session, warnPermille: kit.warnPermille),
      MenuAnchor(
        menuChildren: [
          MenuItemButton(
            key: const Key('add-files'),
            leadingIcon: const Icon(Icons.attach_file, size: 18),
            onPressed: kit.pickFiles == null ? null : () => unawaited(composer._addFiles()),
            child: const Text('Add files or images…'),
          ),
          MenuItemButton(
            key: const Key('add-folder'),
            leadingIcon: const Icon(Icons.folder_outlined, size: 18),
            onPressed: kit.pickFolder == null ? null : () => unawaited(composer._addFolder()),
            child: const Text('Add a folder reference…'),
          ),
          MenuItemButton(
            key: const Key('paste'),
            leadingIcon: const Icon(Icons.content_paste, size: 18),
            onPressed: () => unawaited(composer.paste()),
            child: const Text('Paste from clipboard'),
          ),
        ],
        builder: (context, controller, _) => IconButton(
          key: const Key('composer-plus'),
          tooltip: 'Add to the message',
          visualDensity: VisualDensity.compact,
          onPressed: widget.enabled && store.canAttach(session)
              ? () => controller.isOpen ? controller.close() : controller.open()
              : null,
          icon: const Icon(Icons.add),
        ),
      ),
      if (draft) _SourceChooser(store: store, session: session, kit: kit),
      if (draft) _PlaceToggle(store: store, session: session, kit: kit),
    ];
    final right = <Widget>[
      for (final role in const [OptionRole.model, OptionRole.effort])
        _OptionChooser(store: store, session: session, role: role, options: options, seen: seen),
      if (options.any((option) => roleOf(option) == OptionRole.other))
        _MoreChooser(store: store, session: session, options: options),
      if (draft) _CacheChooser(store: store, session: session),
      if (kit.overlays.isNotEmpty || session.profile != null) _OverlaysChooser(store: store, session: session, kit: kit),
    ];
    final send = widget.working
        ? Row(mainAxisSize: MainAxisSize.min, children: [
            IconButton(tooltip: 'Queue', onPressed: widget.enabled ? widget.onSend : null, icon: const Icon(Icons.schedule_send_outlined)),
            IconButton(tooltip: 'Stop', onPressed: widget.onStop, icon: const Icon(Icons.stop_circle_outlined)),
          ])
        : store.isStarting(session)
        ? const Padding(padding: EdgeInsets.all(12), child: SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2)))
        : IconButton.filled(
            tooltip: 'Send',
            onPressed: widget.enabled ? widget.onSend : null,
            icon: const Icon(Icons.arrow_upward_rounded),
          );
    return LayoutBuilder(builder: (context, box) {
      // On a narrow window the controls go on to a second line, and Send stays at the end.
      if (box.maxWidth < wideFrom) {
        return Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
          Expanded(child: Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 2, runSpacing: 2, children: [...left, ...right])),
          send,
        ]);
      }
      // Wide, it stays on one line: the row is as wide as the composer, and scales down rather than wrap or overflow.
      return FittedBox(
        fit: BoxFit.scaleDown,
        alignment: Alignment.centerLeft,
        child: ConstrainedBox(
          constraints: BoxConstraints(minWidth: box.maxWidth),
          child: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            Row(mainAxisSize: MainAxisSize.min, children: left),
            Row(mainAxisSize: MainAxisSize.min, children: [...right, send]),
          ]),
        ),
      );
    });
  }
}

/// Where a draft's agent comes from: the sources Settings has, each saying what it needs when it cannot run here.
class _SourceChooser extends StatelessWidget {
  const _SourceChooser({required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final source = session.source;
    final problem = source == null ? null : kit.checks[source]?.describe(source.displayName, needs: _needs(source));
    return PopupMenuButton<Object>(
      key: const Key('new-session-source'),
      tooltip: 'Where its agent comes from',
      onSelected: (chosen) => chosen is AgentSource ? store.updateDraft(session, source: chosen) : kit.onSourceSettings?.call(),
      itemBuilder: (context) => [
        for (final each in kit.sources)
          PopupMenuItem(
            key: Key('source ${each.label}'),
            value: each,
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: Icon(sourceIcon(each), size: 18),
              title: Text(each.displayName),
              subtitle: switch (kit.checks[each]) {
                final check? when !check.ready => Text('Needs ${_and(check.missing.isEmpty ? ['setting up'] : check.missing)}',
                    style: TextStyle(color: Theme.of(context).colorScheme.error)),
                _ => null,
              },
            ),
          ),
        if (kit.onSourceSettings != null) ...[
          const PopupMenuDivider(),
          const PopupMenuItem(key: Key('source-settings'), value: #settings, child: Text('Sources in Settings…')),
        ],
      ],
      child: Tooltip(
        message: problem ?? 'Where its agent comes from',
        child: _Pill(
          label: source?.displayName ?? 'Choose a source',
          icon: sourceIcon(source),
          danger: problem != null || source == null,
        ),
      ),
    );
  }

  static List<String> _needs(AgentSource source) => source.command == null ? const [] : commandNeeds(source.command!);
  static String _and(List<String> words) =>
      words.length < 2 ? words.join() : '${words.sublist(0, words.length - 1).join(', ')} and ${words.last}';
}

/// Whether a draft starts in its folder or in a new git worktree of it: Worktree only for a folder in a git repository.
class _PlaceToggle extends StatelessWidget {
  const _PlaceToggle({required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final cwd = session.cwd;
    final inRepository = cwd != null && (kit.inGitRepository?.call(cwd) ?? false);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: SegmentedButton<bool>(
        key: const Key('new-session-place'),
        style: const ButtonStyle(visualDensity: VisualDensity(horizontal: -4, vertical: -4), tapTargetSize: MaterialTapTargetSize.shrinkWrap),
        showSelectedIcon: false,
        segments: [
          const ButtonSegment(value: false, label: Text('Local', style: TextStyle(fontSize: 12.5))),
          ButtonSegment(
            value: true,
            enabled: inRepository,
            tooltip: inRepository ? 'A new git worktree of this folder, on a branch of its own' : 'The folder is not in a git repository',
            label: const Text('Worktree', style: TextStyle(fontSize: 12.5)),
          ),
        ],
        selected: {session.wantsWorktree && inRepository},
        onSelectionChanged: (chosen) => store.updateDraft(session, worktree: chosen.first),
      ),
    );
  }
}

/// The label of [value] in [option], or [value] itself.
String _labelOf(ConfigOption? option, String value, OptionRole role) {
  final choice = option?.choices.where((choice) => choice.value == value).firstOrNull ?? Choice(value, value);
  return choiceLabel(role, choice);
}

String _seenLabel(DateTime? seen, String label) {
  if (seen == null) return label;
  final local = seen.toLocal();
  String two(int n) => n.toString().padLeft(2, '0');
  return '$label, as its agent offered on ${local.year}-${two(local.month)}-${two(local.day)} ${two(local.hour)}:${two(local.minute)}';
}

/// One of the agent's choices in a role, as a menu: before the agent starts, what its source offered last time and Default;
/// after, what it offers. A choice waiting for the next turn says so.
class _OptionChooser extends StatelessWidget {
  const _OptionChooser({required this.store, required this.session, required this.role, required this.options, this.seen});

  final SessionStore store;
  final ChatSession session;
  final OptionRole role;
  final List<ConfigOption> options;
  final DateTime? seen;

  @override
  Widget build(BuildContext context) {
    final option = optionIn(options, role);
    final name = role == OptionRole.model ? 'Model' : 'Effort';
    final pending = session.pending.values.where((choice) => choice.role == role).firstOrNull;
    final draft = session.isDraft;
    if (option == null && pending == null && !draft) return const SizedBox.shrink();
    // Nothing is known of this agent's list until it runs: say so, rather than offer a lone Default.
    if (option == null && pending == null) {
      return Tooltip(
        key: Key('option-${role.name}'),
        message: 'Its ${name.toLowerCase()} choices are listed once the agent has started',
        child: _Pill(label: '$name: after start', icon: role == OptionRole.model ? Icons.memory : Icons.speed, enabled: false),
      );
    }
    final current = pending?.value ?? (draft ? null : option?.current);
    final label = current == null ? (draft ? option?.defaultLabel ?? 'Default' : name) : _labelOf(option, current, role);
    return PopupMenuButton<String?>(
      key: Key('option-${role.name}'),
      tooltip: _seenLabel(draft ? seen : null, name),
      enabled: session.status != SessionStatus.connecting,
      onSelected: (value) {
        if (value == null) {
          // Default takes back the choice waiting, and sets nothing.
          if (pending != null) {
            unawaited(store.choose(session, ConfigOption(id: pending.id, label: name, current: '', choices: const []), null));
          }
        } else if (option != null) {
          unawaited(store.choose(session, option, value));
        }
      },
      itemBuilder: (context) => [
        if (draft) PopupMenuItem<String?>(value: null, child: Text(option?.defaultLabel ?? 'Default')),
        for (final choice in option?.choices ?? const [])
          CheckedPopupMenuItem<String?>(value: choice.value, checked: choice.value == current, child: Text(choiceLabel(role, choice))),
      ],
      child: _Pill(
        label: pending != null && session.status == SessionStatus.working ? '$label (next turn)' : label,
        icon: role == OptionRole.model ? Icons.memory : Icons.speed,
      ),
    );
  }
}

/// The mode the session is in, at the composer's bottom left: the agent's own modes, and Bypass beside them where the agent has
/// none that asks for nothing. Choosing one asks nothing; Shift+Tab in the message cycles them. While Bypass is on it shows in
/// the accent of the scheme as a status.
class _ModeChooser extends StatelessWidget {
  const _ModeChooser({required this.store, required this.session, required this.options, this.seen});

  final SessionStore store;
  final ChatSession session;
  final List<ConfigOption> options;
  final DateTime? seen;

  @override
  Widget build(BuildContext context) {
    if (session.status == SessionStatus.preview) return const SizedBox.shrink();
    final option = modeOptionIn(options);
    final entries = modeEntries(option);
    final current = store.modeOf(session);
    final bypass = store.inBypass(session);
    final pending = session.pending.values.any((choice) => choice.role == OptionRole.mode);
    final shown = entries.where((entry) => entry.value == current).firstOrNull;
    // Canoa's own Bypass sits on the agent's mode, which is said beside it.
    final under = session.bypass ? entries.where((entry) => entry.value == store.underlyingMode(session)).firstOrNull : null;
    final label = under == null ? (shown?.label ?? (current == null ? null : choiceLabel(OptionRole.mode, Choice(current, current))) ?? (session.isDraft ? 'Default' : 'Mode')) : 'Bypass over ${under.label}';
    return PopupMenuButton<Object?>(
      key: const Key('option-mode'),
      tooltip: _seenLabel(session.isDraft ? seen : null, 'Mode (Shift+Tab changes it)'),
      enabled: store.canChangeMode(session) || session.isDraft,
      onSelected: (value) {
        if (value is ModeEntry) {
          unawaited(store.chooseMode(session, value));
        } else if (value == null) {
          store.clearMode(session);
        }
      },
      itemBuilder: (context) => [
        if (session.isDraft) const PopupMenuItem<Object?>(value: null, child: Text('Default')),
        // An agent the app knows nothing of lists its modes once it runs; Canoa's own Bypass works with any.
        if (session.isDraft && option == null)
          const PopupMenuItem<Object?>(enabled: false, child: Text('Its other modes: after it has started')),
        for (final entry in entries)
          CheckedPopupMenuItem<Object?>(
            value: entry,
            enabled: entry.selectable,
            checked: entry.value == current,
            child: Text(entry.label),
          ),
      ],
      child: _Pill(
        label: pending && session.status == SessionStatus.working ? '$label (next turn)' : label,
        icon: bypass ? Icons.fast_forward_outlined : Icons.tune,
        status: bypass,
      ),
    );
  }
}

/// How much of the account's usage allowance is used, beside the mode: the bucket that matters most with its refill, and a menu
/// of every bucket the agent reported. Hidden for an agent that reports none. In the accent of the scheme from the configured
/// fraction used (a status), and in the colour of an error once a bucket is exhausted.
class _UsagePill extends StatelessWidget {
  const _UsagePill({required this.session, required this.warnPermille});

  final ChatSession session;
  final int warnPermille;

  @override
  Widget build(BuildContext context) {
    if (session.status == SessionStatus.preview || !session.reportsUsage) return const SizedBox.shrink();
    final report = session.usageReport;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final headline = report == null ? null : headlineOf(report, now);
    final level = headline == null ? UsageLevel.calm : levelOf(headline, now, warnPermille: warnPermille);
    final text = report == null ? 'No usage yet' : pillText(report, now);
    return MenuAnchor(
      menuChildren: [UsageMenu(report: report, now: now)],
      builder: (context, controller, _) => InkWell(
        key: const Key('usage'),
        borderRadius: BorderRadius.circular(8),
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: 'Usage of the account behind this agent',
          child: _Pill(
            label: text,
            icon: Icons.data_usage,
            danger: level == UsageLevel.danger,
            status: level == UsageLevel.warn,
            maxWidth: 320,
          ),
        ),
      ),
    );
  }
}

/// What the usage control's menu lists: every bucket, the most pressing first, with what is known of it.
class UsageMenu extends StatelessWidget {
  const UsageMenu({super.key, required this.report, required this.now});

  final UsageReport? report;

  /// Unix seconds.
  final int now;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final report = this.report;
    final small = TextStyle(fontSize: 11.5, color: look.muted);
    final buckets = report == null ? const <UsageBucket>[] : ordered(report, now);
    final hit = report?.limit;
    return SelectionArea(
      child: ConstrainedBox(
        constraints: const BoxConstraints(minWidth: 240, maxWidth: 380),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (buckets.isEmpty) Text('Nothing reported yet', key: const Key('usage-empty'), style: small),
              for (final bucket in buckets)
                Padding(
                  key: Key('usage-bucket ${bucket.id}'),
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    Text(
                      bucketLine(bucket, now),
                      style: TextStyle(
                        fontSize: 13,
                        color: switch (levelOf(bucket, now)) {
                          UsageLevel.danger => Theme.of(context).colorScheme.error,
                          UsageLevel.warn => Theme.of(context).colorScheme.tertiary,
                          UsageLevel.calm => null,
                        },
                      ),
                    ),
                    for (final line in bucketDetails(bucket, now)) Text(line, style: small),
                  ]),
                ),
              if (hit != null)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Text(
                    [
                      'Last refusal',
                      if (hit.text != null) hit.text!,
                      if (hit.retryAfter != null) 'retry after ${spanText(hit.retryAfter!.inSeconds)}',
                      if (hit.resetsAt != null && hit.resetsAt! > now) 'back in ${spanText(hit.resetsAt! - now)}',
                    ].join(' · '),
                    style: small,
                  ),
                ),
              if (report?.account case final account?) Text('Account: $account', style: small),
              if (report != null && report.sources.isNotEmpty && !report.sources.contains('reported'))
                Text('The agent reports no figures: these are inferred from its errors', style: small),
            ],
          ),
        ),
      ),
    );
  }
}

/// The agent's other choices, behind one control.
class _MoreChooser extends StatelessWidget {
  const _MoreChooser({required this.store, required this.session, required this.options});

  final SessionStore store;
  final ChatSession session;
  final List<ConfigOption> options;

  @override
  Widget build(BuildContext context) {
    final others = options.where((option) => roleOf(option) == OptionRole.other).toList();
    return PopupMenuButton<(ConfigOption, String)>(
      key: const Key('option-more'),
      tooltip: 'More of the agent\'s choices',
      onSelected: (chosen) => unawaited(store.choose(session, chosen.$1, chosen.$2)),
      itemBuilder: (context) => [
        for (final option in others) ...[
          PopupMenuItem(enabled: false, height: 28, child: Text(option.label, style: const TextStyle(fontSize: 12))),
          for (final choice in option.choices)
            CheckedPopupMenuItem(
              value: (option, choice.value),
              checked: choice.value == (session.pending[option.id]?.value ?? (session.isDraft ? null : option.current)),
              child: Text(choice.label),
            ),
        ],
      ],
      child: const _Pill(label: 'More', icon: Icons.more_horiz),
    );
  }
}

/// Which overlays a draft starts with; once it has started, what its profile gave it.
/// How long a new session's prompt cache lives, for one session: shown only where the source's provider said what it does with
/// the choice, since a source that cannot say has no such control to set. Its sentence is under the composer.
class _CacheChooser extends StatelessWidget {
  const _CacheChooser({required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final source = session.source;
    if (source == null || store.choicesOf(source)?.cacheLifetime == null) return const SizedBox.shrink();
    final asked = store.cacheLifetimeOf(session);
    return PopupMenuButton<CacheLifetime?>(
      key: const Key('option-cache'),
      tooltip: 'How long the prompt cache lives',
      enabled: session.status == SessionStatus.draft,
      onSelected: (value) => store.setCacheLifetime(session, value),
      itemBuilder: (context) => [
        PopupMenuItem<CacheLifetime?>(
          key: const Key('cache-unchosen'),
          value: null,
          child: Text(asked.chosen ? 'Not chosen' : 'Not chosen (${asked.lifetime.label})'),
        ),
        for (final each in CacheLifetime.values)
          CheckedPopupMenuItem<CacheLifetime?>(
            key: Key('cache-${each.id}'),
            value: each,
            checked: asked.chosen && each == asked.lifetime,
            child: Text('${each.label}, ${each.about}'),
          ),
      ],
      child: _Pill(label: 'Cache: ${asked.lifetime.label}', icon: Icons.timer_outlined),
    );
  }
}

class _OverlaysChooser extends StatelessWidget {
  const _OverlaysChooser({required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final on = session.overlays ?? kit.overlays.toSet();
    final count = kit.overlays.where(on.contains).length;
    if (!session.isDraft) {
      return InkWell(
        key: const Key('overlays'),
        borderRadius: BorderRadius.circular(8),
        onTap: kit.onProfile == null ? null : () => kit.onProfile!(session),
        child: Tooltip(
          message: 'What its profile gave it, set when it started',
          child: _Pill(label: 'Overlays${session.overlays == null ? '' : ' ($count)'}', icon: Icons.layers_outlined, enabled: false),
        ),
      );
    }
    return MenuAnchor(
      menuChildren: [
        for (final name in kit.overlays)
          CheckboxMenuButton(
            key: Key('overlay $name'),
            value: on.contains(name),
            closeOnActivate: false,
            onChanged: session.status != SessionStatus.draft
                ? null
                : (value) {
                    final next = {...on};
                    value == true ? next.add(name) : next.remove(name);
                    store.updateDraft(session, overlays: next);
                  },
            child: Text(name),
          ),
      ],
      builder: (context, controller, _) => InkWell(
        key: const Key('overlays'),
        borderRadius: BorderRadius.circular(8),
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: 'The overlays it starts with',
          child: _Pill(label: 'Overlays ($count)', icon: Icons.layers_outlined),
        ),
      ),
    );
  }
}
