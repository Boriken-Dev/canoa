import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../files/viewers/file_sample.dart';
import '../files/viewers/file_viewer.dart';
import '../files/viewers/standard_viewers.dart';
import '../files/viewers/text_viewer.dart' show visibleText;
import '../l10n/app_text.dart';
import '../popout/item_windows.dart';
import '../served/host_files.dart';
import '../session_store.dart';
import '../shell/surface.dart';
import '../theme/look.dart';
import '../workspace/open_in.dart';
import '../workspace/open_in_ui.dart';

export '../files/viewers/file_sample.dart' show sizeText;
export '../files/viewers/image_viewer.dart' show ImagePreview, declaredImageSize, imagePixels, imageSide;
export '../files/viewers/text_viewer.dart' show lineLength, markRun, previewLines, visibleText;

/// The files of the session's workspace, on the host the session runs on, to browse, preview, and edit text files or open loaded
/// session instruction files; the host decides what it shows and grants. It opens before any agent has started.
class FilesSurface extends WorkSurface {
  const FilesSurface(this.filesFor, {this.canWrite, this.viewers});

  static const surfaceId = 'files';

  /// The key under which a [FileReveal] waits in the surface's controller for the panel to show.
  static const revealKey = 'reveal';

  /// The files of the host [session] runs on; null where this app cannot reach that host's files.
  final WorkspaceFiles? Function(ChatSession session) filesFor;

  /// Whether the host currently grants writing regular text files; null defaults to true when [WorkspaceFiles] implements
  /// [WorkspaceFileWriter].
  final bool Function()? canWrite;

  /// The viewers a file is shown with; null gives the panel the ones the app ships, with the person's choices kept only while it runs.
  final ViewerRegistry? viewers;

  @override
  String get id => surfaceId;
  @override
  String get title => 'Files';
  @override
  IconData get icon => Icons.folder_open_outlined;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.right];

  @override
  bool available(ChatSession session) => workspaceOf(session) != null && filesFor(session) != null;

  /// The folder the files are shown from: the session's worktree, else its folder.
  static String? workspaceOf(ChatSession session) => session.worktree?.path ?? session.cwd;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    final root = workspaceOf(session), files = filesFor(session);
    if (root == null || files == null) return const Center(child: Text('This host\'s files cannot be reached from here.'));
    // Reveal and Open in VS Code are for a workspace on this computer, whose paths its file manager and editor know.
    final here = workspaceOpener.workspaceOf(place: session.place, cwd: session.cwd, endpoint: session.endpoint, peerId: session.peerId).target != null;
    final windows = SurfaceEnvironment.maybeOf(context)?.windows;
    Widget panel({FileReveal? reveal, ValueChanged<String>? onOpenWindow}) => FilesPanel(
      files: files,
      root: root,
      opener: here ? workspaceOpener : null,
      reveal: reveal,
      canWrite: canWrite?.call() ?? true,
      viewers: viewers,
      onOpenWindow: onOpenWindow,
    );
    return panel(
      reveal: controller.data[revealKey] as FileReveal?,
      onOpenWindow: windows == null || !windows.available
          ? null
          : (path) {
              // One request, shown once: a new object each time the window is drawn would show the file again.
              final reveal = FileReveal(path);
              unawaited(
                showItemWindow(
                  context,
                  windows,
                  key: windowKey(session, path),
                  session: session.id,
                  title: path.substring(path.lastIndexOf(RegExp(r'[\\/]')) + 1),
                  builder: (_) => panel(reveal: reveal),
                ),
              );
            },
    );
  }

  /// The key of the window that shows [path] of [session]'s workspace.
  static String windowKey(ChatSession session, String path) => 'file:${session.id}:$path';
}

/// A request to show one file: each request is its own object, so asking for the same path again shows it again.
final class FileReveal {
  const FileReveal(this.path, {this.section});

  /// The file's path on the host, absolute, exactly as the host names it: a `#` in it is part of the name.
  final String path;

  /// The heading of a Markdown file to focus, by its anchor or its title; null to open the file as it is.
  final String? section;
}

/// The panel's body: a listing of the workspace, a folder at a time, instruction files loaded for the session, and a file's
/// preview/editor in its place.
class FilesPanel extends StatefulWidget {
  const FilesPanel({super.key, required this.files, required this.root, this.opener, this.reveal, this.canWrite = true, this.viewers, this.onOpenWindow});

  /// Opens a file in a window of its own, given its path; null where there are no windows, which leaves the button out.
  final ValueChanged<String>? onOpenWindow;

  /// The viewers a file can be shown with, and the person's choice among them; null gives the panel its own set of the app's.
  final ViewerRegistry? viewers;

  /// A file to open as soon as the panel shows, and again whenever a new request replaces it.
  final FileReveal? reveal;

  /// What can be done with a path on this computer; null when the files are on another host, where its paths mean nothing here.
  final WorkspaceOpener? opener;

  final WorkspaceFiles files;

  /// The workspace folder as the session names it.
  final String root;

  /// Whether editing is permitted when [files] implements [WorkspaceFileWriter].
  final bool canWrite;

  /// How much of a file a preview asks for.
  static const int previewBytes = 256 * 1024;

  @override
  State<FilesPanel> createState() => _FilesPanelState();
}

class _FilesPanelState extends State<FilesPanel> {
  HostFolderListing? _listing;
  List<HostInstructionFile> _instructions = const [];
  String? _top;
  String? _problem;
  bool _loading = true;
  HostFileRead? _file;
  String? _fileProblem;
  String? _opening;
  bool _editing = false;
  bool _saving = false;
  String? _saveProblem;
  TextEditingController? _editor;
  String _initialEditText = '';

  WorkspaceFileWriter? get _writer => widget.canWrite && widget.files is WorkspaceFileWriter ? widget.files as WorkspaceFileWriter : null;

  /// The heading of a document to open at, asked for by a reveal.
  String? _focusSection;

  ViewerRegistry? _own;

  ViewerRegistry get _viewers => widget.viewers ?? (_own ??= standardViewerRegistry());

  @override
  void initState() {
    super.initState();
    unawaited(_viewers.choices.load());
    _viewers.addListener(_viewersChanged);
    unawaited(_start());
  }

  void _viewersChanged() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    _viewers.removeListener(_viewersChanged);
    _own?.dispose();
    _editor?.dispose();
    super.dispose();
  }

  Future<void> _start() async {
    await Future.wait([_list(widget.root), _loadInstructions()]);
    final reveal = widget.reveal;
    if (reveal != null && mounted) await _reveal(reveal);
  }

  Future<void> _loadInstructions() async {
    final files = widget.files;
    if (files is WorkspaceInstructionFiles) {
      try {
        final found = await (files as WorkspaceInstructionFiles).instructionFiles(widget.root);
        if (mounted) setState(() => _instructions = found);
      } on Object {
        // Optional discovery: folder listing still works if fs/instructions is unsupported or refused.
      }
      return;
    }
    if (mounted && _instructions.isNotEmpty) setState(() => _instructions = const []);
  }

  void _stopEditing() {
    _editor?.dispose();
    _editor = null;
    _editing = false;
    _saving = false;
    _saveProblem = null;
    _initialEditText = '';
  }

  /// Lists the folder holding [reveal]'s file and opens the file, or says it is not there.
  Future<void> _reveal(FileReveal reveal) async {
    final path = reveal.path;
    final cut = path.lastIndexOf(RegExp(r'[\\/]'));
    final name = path.substring(cut + 1);
    setState(() {
      _file = null;
      _stopEditing();
    });
    await _list(cut > 0 ? path.substring(0, cut) : widget.root);
    if (!mounted || widget.reveal != reveal || _problem != null) return;
    final entry = _listing?.entries.where((each) => !each.isFolder && each.name == name).firstOrNull;
    if (entry == null) {
      setState(() => _problem = 'It is not there any more: ${visibleText(path)}');
      return;
    }
    await _open(entry, section: reveal.section);
  }

  @override
  void didUpdateWidget(FilesPanel old) {
    super.didUpdateWidget(old);
    if (old.viewers != widget.viewers) {
      (old.viewers ?? _own)?.removeListener(_viewersChanged);
      _viewers.addListener(_viewersChanged);
      unawaited(_viewers.choices.load());
    }
    if (old.files != widget.files || old.root != widget.root) {
      _top = null;
      _file = null;
      _fileProblem = null;
      _instructions = const [];
      _stopEditing();
      unawaited(_start());
    } else if (widget.reveal != null && widget.reveal != old.reveal) {
      unawaited(_reveal(widget.reveal!));
    }
  }

  String _said(Object error) => error is ControlFailed ? error.message : 'The host could not be reached.';

  Future<void> _list(String path) async {
    setState(() {
      _loading = true;
      _problem = null;
    });
    try {
      final listing = await widget.files.listFolders(path: path, files: true);
      if (!mounted) return;
      setState(() {
        _listing = listing;
        _top ??= listing.path;
        _loading = false;
      });
    } on Object catch (error) {
      if (!mounted) return;
      setState(() {
        _problem = _said(error);
        _loading = false;
      });
    }
  }

  Future<void> _open(HostFolderEntry entry, {String? section}) async {
    if (entry.isFolder) {
      setState(() {
        _file = null;
        _stopEditing();
      });
      return _list(entry.path);
    }
    await _openFilePath(entry.path, entry.name, section: section);
  }

  /// Reads [file] to its end so that what is edited is the whole file.
  Future<void> _loadWhole(WorkspaceFileWriter writer, HostFileRead file) async {
    setState(() => _fileProblem = null);
    try {
      final whole = await writer.readWhole(file.path);
      if (mounted && _file?.path == file.path) setState(() => _file = whole);
    } on Object catch (error) {
      if (mounted) setState(() => _saveProblem = _said(error));
    }
  }

  Future<void> _openFilePath(String path, String name, {String? section}) async {
    setState(() {
      _opening = path;
      _fileProblem = null;
      _focusSection = section;
      _stopEditing();
    });
    try {
      final read = await widget.files.readFile(path, maxBytes: FilesPanel.previewBytes);
      if (!mounted || _opening != path) return;
      setState(() {
        _file = read;
        _opening = null;
      });
    } on Object catch (error) {
      if (!mounted || _opening != path) return;
      setState(() {
        _file = HostFileRead(path: path, name: name, size: 0, kind: 'binary', truncated: false);
        _fileProblem = _said(error);
        _opening = null;
      });
    }
  }

  void _beginEdit(HostFileRead file, List<int> bytes) {
    final text = decodeFileText(bytes);
    _editor?.dispose();
    _editor = TextEditingController(text: text)
      ..addListener(() {
        if (mounted && _editing) setState(() {});
      });
    setState(() {
      _initialEditText = text;
      _saveProblem = null;
      _editing = true;
    });
  }

  Future<void> _saveEdit(HostFileRead file) async {
    final writer = _writer;
    final editor = _editor;
    if (writer == null || editor == null || _saving) return;
    final nextText = editor.text;
    setState(() {
      _saving = true;
      _saveProblem = null;
    });
    try {
      final size = await writer.writeFile(file.path, nextText);
      if (!mounted) return;
      final savedBytes = Uint8List.fromList(utf8.encode(nextText));
      setState(() {
        _file = HostFileRead(
          path: file.path,
          name: file.name,
          size: size,
          kind: 'text',
          truncated: false,
          mime: file.mime,
          bytes: savedBytes,
        );
        _stopEditing();
      });
      unawaited(_loadInstructions());
    } on Object catch (error) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _saveProblem = _said(error);
      });
    }
  }

  /// The workspace actions offered for [path]: Reveal in the file manager, Open in VS Code.
  List<Widget> _actions(BuildContext context, String? path) {
    final opener = widget.opener;
    if (opener == null || path == null) {
      return const [];
    }
    final target = WorkspaceTarget(path, item: true);
    return [
      for (final action in opener.actionsFor(WorkspaceContext.files, target))
        IconButton(
          key: ValueKey('files action ${action.id}'),
          tooltip: action.label,
          icon: Icon(action.icon, size: 18),
          onPressed: () => unawaited(runWorkspaceAction(context, action, target)),
        ),
    ];
  }

  String _where() {
    final here = _listing?.path, top = _top;
    if (here == null || top == null || here == top) return 'Workspace';
    // Above the workspace the whole path is shown: the host lets this key browse the computer.
    if (!here.startsWith(top)) return visibleText(here);
    final cut = here.substring(top.length).replaceAll('\\', '/');
    return visibleText(cut.startsWith('/') ? cut.substring(1) : cut);
  }

  @override
  Widget build(BuildContext context) {
    final file = _file;
    return Material(color: context.look.rail, child: file != null ? _preview(context, file) : _listView(context));
  }

  Widget _listView(BuildContext context) {
    final listing = _listing;
    final muted = TextStyle(color: context.chatLook.muted, fontSize: 12);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(8, 4, 12, 4),
          child: Row(
            children: [
              IconButton(
                key: const ValueKey('files up'),
                tooltip: 'Up one folder',
                icon: const Icon(Icons.arrow_upward, size: 18),
                onPressed: listing == null || listing.parent == null ? null : () => _list(listing.parent!),
              ),
              Expanded(
                child: Text(_where(), key: const ValueKey('files where'), overflow: TextOverflow.ellipsis),
              ),
              ..._actions(context, listing?.path),
              IconButton(
                key: const ValueKey('files refresh'),
                tooltip: 'Read the folder again',
                icon: const Icon(Icons.refresh, size: 18),
                onPressed: listing?.path == null ? null : () => _list(listing!.path!),
              ),
            ],
          ),
        ),
        const Divider(height: 1),
        if (_loading) const LinearProgressIndicator(minHeight: 2),
        Expanded(
          child: _problem != null
              ? Padding(
                  padding: const EdgeInsets.all(16),
                  child: Text(_problem!, key: const ValueKey('files problem')),
                )
              : listing == null
              ? const SizedBox.shrink()
              : listing.entries.isEmpty && _instructions.isEmpty
              ? Center(child: Text('Nothing to show in this folder.', style: muted))
              : ListView(
                  children: [
                    if (_instructions.isNotEmpty && _listing?.path == _top) ...[
                      Padding(
                        padding: const EdgeInsets.fromLTRB(16, 10, 16, 4),
                        child: Text('Instructions', key: const ValueKey('files instructions'), style: muted.copyWith(fontWeight: FontWeight.w600)),
                      ),
                      for (final doc in _instructions)
                        ListTile(
                          key: ValueKey('files instruction ${doc.name}'),
                          dense: true,
                          leading: const Icon(Icons.description_outlined, size: 18),
                          title: Text(visibleText(doc.name), overflow: TextOverflow.ellipsis),
                          subtitle: Text([doc.scope, ...doc.agents].join(' · '), style: muted),
                          trailing: _opening == doc.path
                              ? const SizedBox(width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2))
                              : null,
                          onTap: () => unawaited(_openFilePath(doc.path, doc.name)),
                        ),
                      const Divider(height: 1),
                    ],
                    if (listing.entries.isEmpty)
                      Padding(
                        padding: const EdgeInsets.all(16),
                        child: Text('Nothing else in this folder.', style: muted),
                      ),
                    for (final entry in listing.entries)
                      ListTile(
                        key: ValueKey('files entry ${entry.name}'),
                        dense: true,
                        leading: Icon(entry.isFolder ? Icons.folder_outlined : Icons.insert_drive_file_outlined, size: 18),
                        title: Text(visibleText(entry.name), overflow: TextOverflow.ellipsis),
                        trailing: _opening == entry.path
                            ? const SizedBox(width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2))
                            : null,
                        onTap: () => _open(entry),
                      ),
                    if (listing.more)
                      Padding(
                        padding: const EdgeInsets.all(16),
                        child: Text('More is in this folder than is listed here.', style: muted),
                      ),
                  ],
                ),
        ),
      ],
    );
  }

  Widget _preview(BuildContext context, HostFileRead file) {
    final muted = TextStyle(color: context.chatLook.muted, fontSize: 12);
    final bytes = file.bytes;
    final writer = _writer;
    final dirty = _editing && (_editor?.text ?? '') != _initialEditText;
    final sample = FileSample.of(file);
    final registry = _viewers;
    final type = registry.typeOf(sample);
    final viewer = _fileProblem == null ? registry.choose(sample, type: type, section: _focusSection) : null;
    final options = _fileProblem == null ? [for (final each in registry.matching(sample, type: type)) if (!each.fallback) each] : const <FileViewer>[];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(8, 4, 12, 4),
          child: Row(
            children: [
              IconButton(
                key: const ValueKey('files back'),
                tooltip: 'Back to the folder',
                icon: const Icon(Icons.arrow_back, size: 18),
                onPressed: _saving
                    ? null
                    : () => setState(() {
                        _file = null;
                        _fileProblem = null;
                        _stopEditing();
                      }),
              ),
              Expanded(
                child: Text(visibleText(file.name), key: const ValueKey('files name'), overflow: TextOverflow.ellipsis),
              ),
              if (_editing) ...[
                TextButton(
                  key: const ValueKey('files cancel'),
                  onPressed: _saving ? null : () => setState(_stopEditing),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: 4),
                FilledButton(
                  key: const ValueKey('files save'),
                  onPressed: _saving || !dirty ? null : () => unawaited(_saveEdit(file)),
                  child: Text(_saving ? 'Saving…' : 'Save'),
                ),
              ] else ...[
                if (writer != null && _fileProblem == null && file.kind == 'text' && bytes != null)
                  if (!file.truncated)
                    IconButton(
                      key: const ValueKey('files edit'),
                      tooltip: 'Edit file',
                      icon: const Icon(Icons.edit_outlined, size: 18),
                      onPressed: () => _beginEdit(file, bytes),
                    )
                  else
                    TextButton(
                      key: const ValueKey('files load full'),
                      onPressed: () => unawaited(_loadWhole(writer, file)),
                      child: const Text('Load all to edit'),
                    ),
                if (widget.onOpenWindow != null)
                  IconButton(
                    key: const ValueKey('files window'),
                    tooltip: context.t('window.open'),
                    icon: const Icon(Icons.open_in_new, size: 18),
                    onPressed: () => widget.onOpenWindow!(file.path),
                  ),
                ..._actions(context, file.path),
                if (_fileProblem == null) Text(sizeText(file.size), style: muted),
              ],
            ],
          ),
        ),
        const Divider(height: 1),
        if (!_editing && viewer != null && options.length > 1)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Align(
              alignment: Alignment.centerRight,
              child: SegmentedButton<String>(
                key: const ValueKey('viewer switch'),
                segments: [
                  for (final each in options)
                    ButtonSegment<String>(value: each.id, label: Text(context.t(each.labelKey), key: ValueKey('viewer switch ${each.id}'), style: const TextStyle(fontSize: 11))),
                ],
                selected: {viewer.id},
                showSelectedIcon: false,
                onSelectionChanged: (choice) {
                  setState(() => _focusSection = null);
                  unawaited(registry.choices.choose(type, choice.first));
                },
              ),
            ),
          ),
        if (_saveProblem != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(
              _saveProblem!,
              key: const ValueKey('files save error'),
              style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12),
            ),
          ),
        if (!_editing && file.truncated && bytes != null && file.kind == 'text')
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(
              'Showing the first ${sizeText(bytes.length)} of ${sizeText(file.size)}.',
              key: const ValueKey('files truncated'),
              style: muted,
            ),
          ),
        Expanded(
          child: _editing
              ? Padding(
                  padding: const EdgeInsets.all(16),
                  child: TextField(
                    key: const ValueKey('files editor'),
                    controller: _editor,
                    expands: true,
                    maxLines: null,
                    minLines: null,
                    textAlignVertical: TextAlignVertical.top,
                    style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12),
                    decoration: const InputDecoration(
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                  ),
                )
              : _fileProblem != null
              ? SingleChildScrollView(padding: const EdgeInsets.all(16), child: Text(_fileProblem!, key: const ValueKey('files problem')))
              : viewer == null
              ? const SizedBox.shrink()
              : viewer.build(context, FileViewRequest(sample: sample, type: type, section: _focusSection)),
        ),
      ],
    );
  }
}
