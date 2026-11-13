import 'dart:async';
import 'dart:typed_data';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/material.dart';

import '../acp/local_agent.dart' show InsideFile;
import '../extension/composer_extension.dart';
import '../session_store.dart';
import '../sources/sources.dart';
import 'agent_events.dart' show AgentCommandInfo, MessageRole;
import 'attachments.dart';
import 'composer_controls.dart';
import 'mentions.dart';
import 'session_options.dart';
import 'session_views.dart' show appName;

/// What the composer says while a permission card waits: a message refuses it once and goes to the agent.
const String redirectHint = 'Send a message to decline and redirect the agent';

/// [redirectHint] as it reads on a phone, where the placeholder is one line.
const String narrowRedirectHint = 'Message to decline and redirect';

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
    this.listFolder,
    this.resolveInside,
    this.clipboard = const TextClipboard(),
    this.overlays = const [],
    this.defaultOverlays,
    this.onLoadout,
    this.pasteAsFile = Composer.defaultPasteAsFile,
    this.warnPermille = defaultWarnPermille,
    this.onSourceSettings,
    this.onSettings,
    this.onProfile,
    this.extension,
  });

  /// What the extension adds above the message and to `+`; null outside it.
  final ComposerExtension? extension;

  /// The overlays a new session starts with: Loadout's defaults, which are `overlays` when null.
  final List<String>? defaultOverlays;

  /// Opens Loadout, where an overlay is installed and turned on or off by default; null where there is none.
  final VoidCallback? onLoadout;

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

  /// The files and folders under a folder on this computer, as paths relative to it, and the file or folder one names when it is
  /// inside the folder: what an `@` offers. Null where files cannot be read by their path.
  final Future<List<String>> Function(String folder)? listFolder;
  final InsideFile? Function(String folder, String relative)? resolveInside;

  /// canoa.cfg's overlays, by name, in their order.
  final List<String> overlays;
  final int pasteAsFile;

  /// From this many thousandths used, the usage control shows a status.
  final int warnPermille;

  /// Opens Settings at its sources.
  final VoidCallback? onSourceSettings;

  /// Opens Settings.
  final VoidCallback? onSettings;

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

/// What a session keeps for its next message, as the composer reads and changes it.
final class SessionAttachments implements ComposerAttachments {
  SessionAttachments(this.store, this.session, this.kit);

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  bool get canAttach => store.canAttach(session);

  @override
  List<ComposerChip> get chips => [
    for (final attachment in session.attachments)
      ComposerChip(
        key: attachment,
        name: attachment.name,
        detail: attachment.kind == AttachmentKind.folder ? 'Folder' : sizeLabel(attachment.size),
        kind: switch (attachment.kind) {
          AttachmentKind.image => ChipKind.image,
          AttachmentKind.text => ChipKind.text,
          AttachmentKind.file => ChipKind.file,
          AttachmentKind.folder => ChipKind.folder,
        },
        thumbnail: attachment.isImage ? attachment.bytes : null,
        note: store.refusalOf(session, attachment) ?? store.routeNote(session, attachment),
        refused: store.refusalOf(session, attachment) != null,
      ),
  ];

  @override
  Future<List<String>> addFiles(List<PickedFile> files) async {
    final problems = <String>[];
    for (final file in files) {
      if (file.path.isNotEmpty && kit.statFile != null) {
        problems.addAll(await _addPaths(store, session, [file.path], kit));
      } else {
        final problem = store.addAttachment(session, file.name, await file.read(), mimeType: file.mimeType);
        if (problem != null) problems.add(problem);
      }
    }
    return problems;
  }

  @override
  Future<List<String>> addPaths(List<String> paths) => _addPaths(store, session, paths, kit);

  @override
  String? addImage(Uint8List bytes) {
    var n = 1;
    while (session.attachments.any((item) => item.name == 'Pasted image $n.png')) {
      n++;
    }
    return store.addAttachment(session, 'Pasted image $n.png', bytes, mimeType: 'image/png');
  }

  @override
  String? addText(String text) => store.addText(session, text);

  @override
  String? addFolder(String path) {
    final name = path.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).lastOrNull ?? path;
    return store.addFolder(session, name, path);
  }

  @override
  void remove(Object key) => store.removeAttachment(session, key as Attachment);
}

final _addPaths = addPaths;

/// The message being written, what goes with it, and a row of controls under it: `+`, the source and Local or Worktree
/// before the session starts, the agent's mode, model and effort, the overlays, and Send. The package's [Composer] draws it; what
/// is Canoa's (the controls, the session's attachments, the extension's header) goes in its slots.
class SessionComposer extends StatelessWidget {
  const SessionComposer({
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
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) => _composer(context, narrow: box.maxWidth < Composer.wideFrom),
  );

  /// [narrow] is a phone's width, where the placeholder is one line and so takes its short forms.
  Widget _composer(BuildContext context, {required bool narrow}) {
    final draft = session.status == SessionStatus.draft;
    final options = store.optionsOf(session);
    final seen = store.optionsSeen(session);
    final mentions = kit.listFolder != null && kit.resolveInside != null && store.canAttach(session) && SessionMentions.offeredFor(session)
        ? SessionMentions(store: store, session: session, list: kit.listFolder!, resolve: kit.resolveInside!)
        : null;
    final inputs = [if (mentions != null) '@ files', '/ commands'].join(', ');
    final actions = _actions(session);
    return Composer(
      controller: controller,
      focusNode: focusNode,
      onSend: onSend,
      enabled: enabled,
      working: working,
      starting: store.isStarting(session),
      onStop: onStop,
      commands: commands,
      actions: actions,
      actionsTitle: '$appName actions',
      mentions: mentions,
      centred: centred,
      hint: !enabled
          ? (store.isStarting(session) ? 'Starting ${session.agent}…' : 'Connect an ACP peer to chat')
          : session.pendingPermissions.isNotEmpty
          ? (narrow ? narrowRedirectHint : redirectHint)
          : session.isDraft
          ? narrow
                ? 'Message ${session.source?.displayName ?? 'an agent'}: $inputs'
                : 'Message ${session.source?.displayName ?? 'an agent'}: it starts when you send. $inputs'
          : 'Message agent: $inputs',
      header: kit.extension?.header(context, session, controller),
      attachments: SessionAttachments(store, session, kit),
      pickFiles: kit.pickFiles == null
          ? null
          : () async => [
              for (final file in await kit.pickFiles!())
                (name: file.name, path: file.path, mimeType: file.mimeType, read: file.readAsBytes),
            ],
      pickFolder: kit.pickFolder,
      clipboard: kit.clipboard,
      pasteAsFile: kit.pasteAsFile,
      plusItems: [
        ...?kit.extension?.menuItems(context, session),
        MenuItemButton(
          key: const Key('plus-slash'),
          leadingIcon: const Icon(Icons.terminal, size: 18),
          // Commands are read from the start of the message, so the entry is there for an empty one.
          onPressed: controller.text.isEmpty
              ? () {
                  controller.value = const TextEditingValue(text: '/', selection: TextSelection.collapsed(offset: 1));
                  focusNode?.requestFocus();
                }
              : null,
          child: const Text('Slash commands'),
        ),
      ],
      onShiftTab: session.status == SessionStatus.preview ? null : () => unawaited(store.cycleMode(session)),
      recall: _lastSent,
      // The mode first at the bottom left, then Add; the model and effort sit beside Send, as Claude Code and Codex lay them out.
      before: [
        ModeChooser(store: store, session: session, options: options, seen: seen),
        if (session.status != SessionStatus.preview && session.reportsUsage)
          UsagePill(report: session.usageReport, warnPermille: kit.warnPermille),
      ],
      after: [
        if (draft) SourceChooser(store: store, session: session, kit: kit),
        if (draft) PlaceToggle(store: store, session: session, kit: kit),
      ],
      trailing: [
        for (final role in const [OptionRole.model, OptionRole.effort])
          OptionChooser(store: store, session: session, role: role, options: options, seen: seen),
        if (options.any((option) => roleOf(option) == OptionRole.other)) MoreChooser(store: store, session: session, options: options),
        if (draft) CacheChooser(store: store, session: session),
        if (kit.overlays.isNotEmpty || session.profile != null) OverlaysChooser(store: store, session: session, kit: kit),
      ],
    );
  }

  /// Canoa's own entries in the `/` list: what the person could do from the controls, by name.
  List<ComposerAction> _actions(ChatSession session) => [
    if (session.status != SessionStatus.preview)
      ComposerAction(name: 'mode', description: 'Switch to the next mode', run: () => unawaited(store.cycleMode(session))),
    if (kit.onSettings != null) ComposerAction(name: 'settings', description: 'Open Settings', run: kit.onSettings!),
    if (kit.onSourceSettings != null) ComposerAction(name: 'sources', description: 'Open the source settings', run: kit.onSourceSettings!),
    if (kit.onLoadout != null) ComposerAction(name: 'loadout', description: 'Open Loadout', run: kit.onLoadout!),
    if (kit.onProfile != null && session.profile != null)
      ComposerAction(name: 'profile', description: 'Show what this session was given', run: () => kit.onProfile!(session)),
  ];

  /// The last message the person sent here, which Up in an empty message brings back.
  String? _lastSent() {
    for (final message in session.messages.reversed) {
      if (message.role == MessageRole.user) return message.text;
    }
    return null;
  }
}
