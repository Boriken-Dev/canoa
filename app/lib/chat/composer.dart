import 'dart:async';
import 'dart:typed_data';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/material.dart';

import '../accounts/account_chooser.dart';
import '../accounts/accounts_controller.dart';
import '../acp/local_agent.dart' show InsideFile;
import '../history/agent_switch.dart' show PortableAgents;
import '../config/canoa_config.dart' show Preset, ResolvedLoadout;
import '../extension/composer_extension.dart';
import '../session_store.dart';
import '../sources/sources.dart';
import '../store/programs.dart' show ProgramStores;
import '../workspace/remote_host.dart' show HostChoice;
import 'agent_events.dart' show AgentCommandInfo, MessageRole;
import 'attachments.dart';
import '../voice/reply_reader.dart';
import 'dictation.dart';
import 'palette.dart' show dictationKeys;
import 'reply_voice.dart';
import 'composer_controls.dart';
import 'source_trouble.dart';
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
    this.uploadFile,
    this.hostFiles,
    this.listFolder,
    this.resolveInside,
    this.foldersReadable,
    this.remoteFolders,
    this.foldersChanged,
    this.clipboard = const TextClipboard(),
    this.overlays = const [],
    this.defaultOverlays,
    this.loadouts = const {},
    this.defaultLoadout,
    this.onLoadout,
    this.presets = const {},
    this.savePreset,
    this.onSetup,
    this.onProfiles,
    this.pasteAsFile = Composer.defaultPasteAsFile,
    this.enterSteers = false,
    this.warnPermille = defaultWarnPermille,
    this.onSourceSettings,
    this.onSettings,
    this.onProfile,
    this.onDelegate,
    this.accounts,
    this.accountsOf,
    this.onAccountSettings,
    this.switchAccount,
    this.switchAgent,
    this.portable,
    this.programs,
    this.extension,
    this.dictation,
    this.reader,
    this.onVoiceMode,
    this.hosts = const [],
    this.chooseHost,
    this.addHost,
    this.manageHosts,
    this.browseFolder,
    this.nativeFolder,
    this.sourcesOf,
    this.recentFoldersOf,
    this.repositoryOf,
    this.pathUnchecked,
  });

  /// Where a new session can start: this app's own host first, then each saved connection. Empty where there is a choice of none,
  /// which hides the host chip.
  final List<HostChoice> hosts;

  /// Aims [session], a draft, at the saved connection [name], or at this app's own host for null; the agents, accounts and recent
  /// folders it offers then come from that host.
  final void Function(ChatSession session, String? name)? chooseHost;

  /// Opens Add connection, and Hosts; each returns to the same draft.
  final Future<void> Function()? addHost;
  final VoidCallback? manageHosts;

  /// Asks which folder [session] works in through its host's own folder listing; null where [pickFolder] is all there is.
  final Future<String?> Function(ChatSession session)? browseFolder;

  /// This computer's own folder dialog, a shortcut for a draft on Local; null where there is none.
  final Future<String?> Function()? nativeFolder;

  /// The sources [session] can start from, which are its host's; [sources] when null.
  final List<AgentSource> Function(ChatSession session)? sourcesOf;

  /// The folders sessions worked in on [session]'s host, newest first; [recentFolders] when null.
  final List<String> Function(ChatSession session)? recentFoldersOf;

  /// Whether [session]'s folder is in a git repository on its host, when that is not the host [inGitRepository] asks.
  final bool? Function(ChatSession session)? repositoryOf;

  /// Whether a path [session] is given is taken as typed, with nothing on its host to check it against (a direct ACP endpoint).
  final bool Function(ChatSession session)? pathUnchecked;

  List<AgentSource> sourcesFor(ChatSession session) => sourcesOf?.call(session) ?? sources;

  List<String> recentFor(ChatSession session) => recentFoldersOf?.call(session) ?? recentFolders;

  /// Dictation into the message; null where this build has no way to record, which leaves the microphone out.
  final Dictation? dictation;

  /// Reads replies aloud; null where this build has no voice to read with, which leaves voice mode's switch and the play buttons out.
  final ReplyReader? reader;

  /// Told when voice mode is turned on or off in the composer, to keep the choice.
  final ValueChanged<bool>? onVoiceMode;

  /// Whether Enter steers rather than queues while a turn runs (`[sessions] while_working`); Alt+Enter does the other.
  final bool enterSteers;

  /// What the extension adds above the message and to `+`; null outside it.
  final ComposerExtension? extension;

  /// The overlays a new session starts with: Loadout's defaults, which are `overlays` when null.
  final List<String>? defaultOverlays;

  /// canoa.cfg's loadouts as a session would start with each, by name; a session picks one in the composer.
  final Map<String, ResolvedLoadout> loadouts;

  /// The loadout a session that picks none starts with (`[defaults] loadout`); null when none is set.
  final String? defaultLoadout;

  /// Opens the Store, where an overlay is installed and turned on or off by default; null where there is none.
  final VoidCallback? onLoadout;

  /// canoa.cfg's `[profiles]` by name: the values a new session may start with, chosen in its footer.
  final Map<String, Preset> presets;

  /// Writes the profile [name] into canoa.cfg as [preset], a new one or one that is changed; false when it could not be written.
  /// Null where canoa.cfg is not the person's to write, which offers no saving.
  final Future<bool> Function(String name, Preset preset)? savePreset;

  /// Shows the setup [session] starts with, layer by layer; null where it cannot be worked out.
  final void Function(ChatSession session)? onSetup;

  /// Opens Settings at the profiles; null where there is none.
  final VoidCallback? onProfiles;

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

  /// Keeps a picked file on the computer the agent runs on and says where, for a page that has no path to give: anything but a
  /// small image then goes as that file, where it would otherwise have to be text embedded in the message. Null where a picked
  /// file is read as it is.
  final Future<({String path, int size})> Function(String name, List<int> bytes)? uploadFile;

  /// Whether [pickFolder] and [uploadFile] reach the computer [session]'s agent runs on: false for a session on another peer, whose
  /// agent could not read a folder or file of this page's host. Null means they do for every session.
  final bool Function(ChatSession session)? hostFiles;
  final ClipboardReader clipboard;

  /// The files and folders under a folder on this computer, as paths relative to it, and the file or folder one names when it is
  /// inside the folder: what an `@` offers. Null where files cannot be read by their path.
  final Future<List<String>> Function(String folder)? listFolder;
  final Future<InsideFile?> Function(String folder, String relative)? resolveInside;

  /// Whether [session]'s folder is on a computer this app reads folders of (for `@` names); null means this one, for a session
  /// started here.
  final bool Function(ChatSession session)? foldersReadable;

  /// The folder services of the host [session] runs on when that is another Canoa host, for its @ names: that host lists its own
  /// folders, and this computer's are not read. Null for a session on no other host.
  final FolderServices? Function(ChatSession session)? remoteFolders;

  /// Told when a folder's answer arrives that a widget built with a guess.
  final Listenable? foldersChanged;

  /// canoa.cfg's overlays, by name, in their order.
  final List<String> overlays;
  final int pasteAsFile;

  /// From this many thousandths used, the usage control shows a status.
  final int warnPermille;

  /// Opens Settings at its sources.
  final VoidCallback? onSourceSettings;

  /// Opens Settings.
  final VoidCallback? onSettings;

  /// The programs the host's Store keeps, when it grants installing them: where a source whose program is missing is put right.
  final ProgramStores? programs;

  /// The accounts the host knows, for the chip that names the one a session runs on; null where the app has none.
  final AccountsController? accounts;

  /// The accounts of the saved connection a session was aimed at, null while that host has told none; used instead of [accounts]
  /// for a session that has a host.
  final AccountsController? Function(ChatSession session)? accountsOf;

  /// The accounts the chip and picker list for [session] (any session when null): its own host's when it is aimed at one.
  AccountsController? accountsFor(ChatSession? session) => session?.host == null ? accounts : accountsOf?.call(session!);

  /// Opens Settings at Accounts.
  final VoidCallback? onAccountSettings;

  /// Moves a running session to another account, ending its agent and starting one on that account that opens the same
  /// conversation; answers whether it moved. Null where the app cannot start agents.
  final Future<bool> Function(ChatSession session, String account)? switchAccount;

  /// Moves a running session to another agent of this computer: the host exports the conversation and imports it into that agent,
  /// which opens it; answers whether it moved. Null where the app cannot do that.
  final Future<bool> Function(ChatSession session, AgentSource source)? switchAgent;

  /// What each of the host's sources can export and import, which says which agents a running conversation can go to.
  final PortableAgents? portable;

  /// Shows what a session was given of its profile.
  final void Function(ChatSession session)? onProfile;

  /// Opens the dialog in which the person hands a task to another agent; null where the app has none.
  final void Function(ChatSession session)? onDelegate;
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
        try {
          final bytes = await file.read();
          final upload = kit.hostFiles?.call(session) == false ? null : kit.uploadFile;
          final String? problem;
          if (upload != null && !(imageTypeOf(file.name, file.mimeType) != null && bytes.length <= maxAttachedImage)) {
            if (bytes.isEmpty) {
              problem = '${file.name} is empty.';
            } else {
              final kept = await upload(file.name, bytes);
              problem = store.addFile(session, file.name, kept.path, kept.size, mimeType: file.mimeType);
            }
          } else {
            problem = await store.addAttachmentFitted(session, file.name, bytes, mimeType: file.mimeType);
          }
          if (problem != null) problems.add(problem);
        } on Object catch (error) {
          problems.add('${file.name}: $error');
        }
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

/// The message being written, in one box: `+` at the left, the text, what goes with it as cards above it, and at the right the
/// microphone while it is empty and Send once it is not. Under the box a quiet line holds the model and effort, the mode, the
/// account and, for a draft, the source and the rest of what it starts with. The package's [Composer] draws it; what is Canoa's
/// (the controls, the session's attachments, the extension's header) goes in its slots.
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
    this.onSteer,
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

  /// Stops the turn that runs and sends the message first; null offers no Steer.
  final VoidCallback? onSteer;
  final List<AgentCommandInfo> commands;

  /// Whether it is the middle of a new session's empty view rather than docked under a conversation.
  final bool centred;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) => Actions(
      actions: _fieldOwnsVerticalMoves,
      // A dictation that is under way keeps its microphone in view, though text has been typed beside it.
      child: ListenableBuilder(
        listenable: kit.dictation ?? _nothing,
        builder: (context, _) => _composer(context, narrow: box.maxWidth < Composer.wideFrom),
      ),
    ),
  );

  static final Listenable _nothing = Listenable.merge(const []);

  /// Up and Down, and Page Up and Page Down, in the field are the field's: the selection area around the screen has actions for
  /// the same intents, and a field that defers to it can be handed an action that is not its own. Each is answered by handing the
  /// intent back to the field's default.
  static final Map<Type, Action<Intent>> _fieldOwnsVerticalMoves = {
    ExtendSelectionVerticallyToAdjacentLineIntent: _FieldMove<ExtendSelectionVerticallyToAdjacentLineIntent>(),
    ExtendSelectionVerticallyToAdjacentPageIntent: _FieldMove<ExtendSelectionVerticallyToAdjacentPageIntent>(),
  };

  /// [narrow] is a phone's width, where the placeholder is one line and so takes its short forms.
  Widget _composer(BuildContext context, {required bool narrow}) {
    final draft = session.status == SessionStatus.draft;
    final options = store.optionsOf(session);
    final seen = store.optionsSeen(session);
    final remote = kit.remoteFolders?.call(session);
    final inFolder = session.cwd?.isNotEmpty ?? false
        ? remote != null
              ? SessionMentions(store: store, session: session, list: remote.list, resolve: remote.resolve, scope: session.host ?? '')
              : kit.listFolder != null && kit.resolveInside != null && SessionMentions.offeredFor(session, readable: kit.foldersReadable)
              ? SessionMentions(store: store, session: session, list: kit.listFolder!, resolve: kit.resolveInside!)
              : null
        : null;
    // An `@` also names the other conversations, which need no folder; only a session that can take attachments has any.
    final atMentions = store.canAttach(session) ? SessionAtMentions(store: store, session: session, files: inFolder) : null;
    final mentions = atMentions != null && (inFolder != null || atMentions.conversationsFor('').isNotEmpty) ? atMentions : null;
    final actions = _actions(session);
    final dictation = kit.dictation;
    return Composer(
      controller: controller,
      focusNode: focusNode,
      onSend: onSend,
      enabled: enabled,
      working: working,
      starting: store.isStarting(session),
      onStop: onStop,
      onSteer: onSteer,
      enterSteers: kit.enterSteers,
      commands: commands,
      actions: actions,
      actionsTitle: '$appName actions',
      // / lists everything the session has, scrolling past what fits.
      suggestionLimit: 200,
      mentions: mentions,
      centred: centred,
      hint: _hint(narrow: narrow),
      header: kit.extension?.header(context, session, controller),
      attachments: SessionAttachments(store, session, kit),
      pickFiles: kit.pickFiles == null
          ? null
          : () async => [
              for (final file in await kit.pickFiles!())
                (name: file.name, path: file.path, mimeType: file.mimeType, read: file.readAsBytes),
            ],
      pickFolder: kit.hostFiles?.call(session) == false ? null : kit.pickFolder,
      clipboard: kit.clipboard,
      pasteAsFile: kit.pasteAsFile,
      plusItems: [
        ...?kit.extension?.menuItems(context, session),
        if (mentions != null)
          MenuItemButton(
            key: const Key('plus-mention'),
            leadingIcon: const Icon(Icons.alternate_email, size: 18),
            // A mention is read at the start of a word, so the entry adds one to the end.
            onPressed: () {
              final text = controller.text;
              final at = text.isEmpty || RegExp(r'\s$').hasMatch(text) ? '@' : ' @';
              controller.value = TextEditingValue(text: '$text$at', selection: TextSelection.collapsed(offset: text.length + at.length));
              focusNode?.requestFocus();
            },
            child: Text(inFolder != null ? 'Mention a file' : 'Mention a conversation'),
          ),
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
        // The microphone gives way to Send once something is typed, so dictating into a message goes on from here (or the shortcut).
        if (dictation != null)
          MenuItemButton(
            key: const Key('plus-dictate'),
            leadingIcon: const Icon(Icons.mic_none, size: 18),
            onPressed: dictation.cancellable ? null : () => unawaited(dictation.start()),
            child: Text('Dictate ($dictationKeys)'),
          ),
        if (kit.reader case final reader? when reader.available) VoiceModeMenuItem(reader: reader, onChanged: kit.onVoiceMode ?? (_) {}),
      ],
      onShiftTab: session.status == SessionStatus.preview ? null : () => unawaited(store.cycleMode(session)),
      recall: _lastSent,
      primary: dictation == null ? null : DictationButton(dictation: dictation, controller: controller, shortcut: dictationKeys),
      holdPrimary: dictation != null && dictation.state != DictationState.idle,
      // Quiet text under the box, the choices a person changes most first; a draft's extra choosers follow in the same line.
      below: [
        ModelEffortChooser(store: store, session: session, options: options, kit: kit, seen: seen),
        ModeChooser(store: store, session: session, options: options, seen: seen),
        AccountChooser(store: store, session: session, kit: kit),
        if (draft) SourceChooser(store: store, session: session, kit: kit),
        if (draft) SourceTroubleChip(store: store, session: session, kit: kit),
        if (draft && session.kind == SessionKind.workspace) PlaceToggle(store: store, session: session, kit: kit),
        if (draft && (kit.presets.isNotEmpty || kit.savePreset != null)) PresetChooser(store: store, session: session, kit: kit),
        if (options.any((option) => roleOf(option) == OptionRole.other)) MoreChooser(store: store, session: session, options: options),
        if (draft) CacheChooser(store: store, session: session),
        if (kit.overlays.isNotEmpty || kit.loadouts.isNotEmpty || session.profile != null) OverlaysChooser(store: store, session: session, kit: kit),
        if (session.status != SessionStatus.preview && session.reportsUsage)
          UsagePill(report: session.usageReport, warnPermille: kit.warnPermille),
        if (kit.reader case final reader? when reader.available)
          VoiceModeStatus(reader: reader, controller: controller, dictation: kit.dictation, onChanged: kit.onVoiceMode ?? (_) {}),
        if (working) const _Busy(),
      ],
    );
  }

  /// What the message field says while it is empty: who it goes to and the keys that send it.
  String _hint({required bool narrow}) {
    if (!enabled) return store.isStarting(session) ? 'Starting ${session.agent}…' : 'Connect to an agent to chat';
    if (session.pendingPermissions.isNotEmpty) return narrow ? narrowRedirectHint : redirectHint;
    final name = session.isDraft ? (session.source?.displayName ?? 'an agent') : (shortLine(session.agent) ?? 'the agent');
    if (narrow) return 'Message $name';
    final keys = !working
        ? 'Enter to send'
        : onSteer == null
        ? 'Enter to queue'
        : kit.enterSteers
        ? 'Enter to steer, Alt+Enter to queue'
        : 'Enter to queue, Alt+Enter to steer';
    return 'Message $name \u00b7 $keys';
  }

  /// Canoa's own entries in the `/` list: what the person could do from the controls, by name.
  List<ComposerAction> _actions(ChatSession session) => [
    ..._givenActions(session),
    if (session.status != SessionStatus.preview)
      ComposerAction(name: 'mode', description: 'Switch to the next mode', run: () => unawaited(store.cycleMode(session))),
    if (kit.onSettings != null) ComposerAction(name: 'settings', description: 'Open Settings', run: kit.onSettings!),
    if (kit.onSourceSettings != null) ComposerAction(name: 'sources', description: 'Open the source settings', run: kit.onSourceSettings!),
    if (kit.onLoadout != null) ComposerAction(name: 'loadout', description: 'Open Loadouts', run: kit.onLoadout!),
    if (kit.onDelegate != null && store.canDelegate(session))
      ComposerAction(name: 'delegate', description: 'Hand a task to another agent', run: () => kit.onDelegate!(session)),
    if (kit.onProfile != null && session.profile != null)
      ComposerAction(name: 'profile', description: 'Show what this session was given', run: () => kit.onProfile!(session)),
  ];

  /// The skills and MCP servers Canoa gave the session, which the agent may not list as commands: a pick writes a request for the
  /// agent to use it, to finish and send. A skill the agent lists under its own commands is not listed twice.
  List<ComposerAction> _givenActions(ChatSession session) {
    final given = session.profile?.applied;
    if (given == null) return const [];
    final listed = {
      for (final command in commands) ...[command.name, command.name.substring(command.name.lastIndexOf(':') + 1)],
    };
    ComposerAction offer(String name, String group, String description, String request) => ComposerAction(
      name: name,
      description: description,
      group: group,
      run: () {
        controller.value = TextEditingValue(text: request, selection: TextSelection.collapsed(offset: request.length));
        focusNode?.requestFocus();
      },
    );
    return [
      for (final item in given)
        if (item.key.startsWith('skills.') && !listed.contains(item.key.substring('skills.'.length)))
          offer(item.key.substring('skills.'.length), 'Skills given to this session', 'Ask the agent to use this skill',
              'Use the ${item.key.substring('skills.'.length)} skill to '),
      for (final item in given)
        if (item.key.startsWith('mcp.') && !item.key.substring('mcp.'.length).contains('.'))
          offer(item.key.substring('mcp.'.length), 'MCP servers given to this session', 'Ask the agent to use this server\'s tools',
              'Use the ${item.key.substring('mcp.'.length)} MCP server to '),
    ];
  }

  /// The last message the person sent here, which Up in an empty message brings back.
  String? _lastSent() {
    for (final message in session.messages.reversed) {
      if (message.role == MessageRole.user) return message.text;
    }
    return null;
  }
}

/// The field's own action for [T], reached through the overridable action the field asks about.
final class _FieldMove<T extends Intent> extends ContextAction<T> {
  @override
  Object? invoke(T intent, [BuildContext? context]) => callingAction?.invoke(intent);
}

/// A small spinner at the end of the line under the box while a turn runs.
class _Busy extends StatelessWidget {
  const _Busy();

  @override
  Widget build(BuildContext context) => const Padding(
    padding: EdgeInsets.symmetric(horizontal: 8),
    child: SizedBox.square(key: Key('composer-busy'), dimension: 12, child: CircularProgressIndicator(strokeWidth: 1.5)),
  );
}
