import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../served/host_files.dart';
import '../session_store.dart';
import '../shell/surface.dart';
import '../theme/look.dart';
import '../workspace/open_in.dart';
import '../workspace/open_in_ui.dart';

/// The files of the session's workspace, on the host the session runs on, to browse and preview. Read-only: nothing here writes, and
/// the host decides what it shows (its own roots, its protected paths). It opens before any agent has started.
class FilesSurface extends WorkSurface {
  const FilesSurface(this.filesFor);

  static const surfaceId = 'files';

  /// The key under which a [FileReveal] waits in the surface's controller for the panel to show.
  static const revealKey = 'reveal';

  /// The files of the host [session] runs on; null where this app cannot reach that host's files.
  final WorkspaceFiles? Function(ChatSession session) filesFor;

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
    return FilesPanel(files: files, root: root, opener: here ? workspaceOpener : null, reveal: controller.data[revealKey] as FileReveal?);
  }
}

/// [text] with every control, format, bidirectional or invisible character but a line break or tab shown as `<U+XXXX>`, so what a
/// person reads is what the file says; and a run of combining marks cut after [markRun], the rest shown the same way. The host leaves
/// such names out of its listings; this is for what it cannot know, the text.
String visibleText(String text) {
  final out = StringBuffer();
  var run = 0;
  for (final rune in text.runes) {
    final combining = _combines(rune);
    run = combining ? run + 1 : 0;
    if (_hides(rune) || (combining && run > markRun)) {
      out.write('<U+${rune.toRadixString(16).toUpperCase().padLeft(4, '0')}>');
    } else {
      out.writeCharCode(rune);
    }
  }
  return out.toString();
}

/// How many combining marks may follow one character before the rest are shown as code points: enough for any language, too few to
/// stack a line of text out of its row.
const int markRun = 3;

/// Whether [c] is a combining mark.
bool _combines(int c) =>
    (c >= 0x0300 && c <= 0x036f) ||
    (c >= 0x0483 && c <= 0x0489) ||
    (c >= 0x0591 && c <= 0x05bd) ||
    (c >= 0x064b && c <= 0x065f) ||
    (c >= 0x1ab0 && c <= 0x1aff) ||
    (c >= 0x1dc0 && c <= 0x1dff) ||
    (c >= 0x20d0 && c <= 0x20ff) ||
    (c >= 0xfe20 && c <= 0xfe2f);

/// The characters the host's own list of hidden text names (`store::text`), and the control characters but a line break, a carriage
/// return and a tab.
bool _hides(int c) =>
    c <= 0x08 ||
    c == 0x0b ||
    c == 0x0c ||
    (c >= 0x0e && c <= 0x1f) ||
    (c >= 0x7f && c <= 0x9f) ||
    c == 0xad ||
    c == 0x034f ||
    c == 0x061c ||
    c == 0x115f ||
    c == 0x1160 ||
    (c >= 0x17b4 && c <= 0x17b5) ||
    c == 0x180e ||
    (c >= 0x200b && c <= 0x200f) ||
    c == 0x2028 ||
    c == 0x2029 ||
    (c >= 0x202a && c <= 0x202e) ||
    (c >= 0x2060 && c <= 0x206f) ||
    c == 0x3164 ||
    (c >= 0xfe00 && c <= 0xfe0f) ||
    c == 0xfeff ||
    c == 0xffa0 ||
    (c >= 0xfff9 && c <= 0xfffb) ||
    (c >= 0x1d173 && c <= 0x1d17a) ||
    (c >= 0xe0000 && c <= 0xe007f) ||
    (c >= 0xe0100 && c <= 0xe01ef);

/// The lines of [text] as shown: each cut into pieces of at most [lineLength] characters, so no one line is a layout of the whole
/// file, with a carriage return before a line break dropped.
List<String> previewLines(String text) {
  final lines = <String>[];
  for (final line in text.split('\n')) {
    final bare = line.endsWith('\r') ? line.substring(0, line.length - 1) : line;
    if (bare.length <= lineLength) {
      lines.add(bare);
      continue;
    }
    var at = 0;
    while (at < bare.length) {
      var end = at + lineLength > bare.length ? bare.length : at + lineLength;
      // Not between the halves of one character.
      if (end < bare.length && bare.codeUnitAt(end) >= 0xdc00 && bare.codeUnitAt(end) <= 0xdfff) {
        end--;
      }
      lines.add(bare.substring(at, end));
      at = end;
    }
  }
  return lines;
}

/// The longest piece of a line the preview lays out at once.
const int lineLength = 400;

/// The most pixels an image may declare for the preview to decode it: a small file can claim dimensions that take gigabytes to draw.
const int imagePixels = 16 * 1000 * 1000;

/// The longest side an image is decoded at; a larger one is scaled down as it is read.
const int imageSide = 1600;

/// What a size in bytes reads as.
String sizeText(int bytes) {
  if (bytes < 1024) return '$bytes bytes';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(bytes < 10 * 1024 ? 1 : 0)} KiB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MiB';
}

/// A request to show one file: each request is its own object, so asking for the same path again shows it again.
final class FileReveal {
  const FileReveal(this.path);

  /// The file's path on the host, absolute.
  final String path;
}

/// The panel's body: a listing of the workspace, a folder at a time, and a file's preview in its place.
class FilesPanel extends StatefulWidget {
  const FilesPanel({super.key, required this.files, required this.root, this.opener, this.reveal});

  /// A file to open as soon as the panel shows, and again whenever a new request replaces it.
  final FileReveal? reveal;

  /// What can be done with a path on this computer; null when the files are on another host, where its paths mean nothing here.
  final WorkspaceOpener? opener;

  final WorkspaceFiles files;

  /// The workspace folder as the session names it.
  final String root;

  /// How much of a file a preview asks for.
  static const int previewBytes = 256 * 1024;

  @override
  State<FilesPanel> createState() => _FilesPanelState();
}

class _FilesPanelState extends State<FilesPanel> {
  HostFolderListing? _listing;
  String? _top;
  String? _problem;
  bool _loading = true;
  HostFileRead? _file;
  String? _fileProblem;
  String? _opening;

  @override
  void initState() {
    super.initState();
    unawaited(_start());
  }

  Future<void> _start() async {
    await _list(widget.root);
    final reveal = widget.reveal;
    if (reveal != null && mounted) await _reveal(reveal);
  }

  /// Lists the folder holding [reveal]'s file and opens the file, or says it is not there.
  Future<void> _reveal(FileReveal reveal) async {
    final path = reveal.path;
    final cut = path.lastIndexOf(RegExp(r'[\\/]'));
    final name = path.substring(cut + 1);
    setState(() => _file = null);
    await _list(cut > 0 ? path.substring(0, cut) : widget.root);
    if (!mounted || widget.reveal != reveal || _problem != null) return;
    final entry = _listing?.entries.where((each) => !each.isFolder && each.name == name).firstOrNull;
    if (entry == null) {
      setState(() => _problem = 'It is not there any more: ${visibleText(path)}');
      return;
    }
    await _open(entry);
  }

  @override
  void didUpdateWidget(FilesPanel old) {
    super.didUpdateWidget(old);
    if (old.files != widget.files || old.root != widget.root) {
      _top = null;
      _file = null;
      _fileProblem = null;
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

  Future<void> _open(HostFolderEntry entry) async {
    if (entry.isFolder) {
      setState(() => _file = null);
      return _list(entry.path);
    }
    setState(() {
      _opening = entry.path;
      _fileProblem = null;
    });
    try {
      final read = await widget.files.readFile(entry.path, maxBytes: FilesPanel.previewBytes);
      if (!mounted || _opening != entry.path) return;
      setState(() {
        _file = read;
        _opening = null;
      });
    } on Object catch (error) {
      if (!mounted || _opening != entry.path) return;
      setState(() {
        _file = HostFileRead(path: entry.path, name: entry.name, size: 0, kind: 'binary', truncated: false);
        _fileProblem = _said(error);
        _opening = null;
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
              : listing.entries.isEmpty
              ? Center(child: Text('Nothing to show in this folder.', style: muted))
              : ListView(
                  children: [
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
    Widget body;
    List<String>? lines;
    if (_fileProblem != null) {
      body = Text(_fileProblem!, key: const ValueKey('files problem'));
    } else if (file.kind == 'text' && bytes != null) {
      lines = previewLines(_decode(bytes));
      body = const SizedBox.shrink();
    } else if (file.kind == 'image' && bytes != null) {
      body = ImagePreview(key: ValueKey('image ${file.path}'), bytes: bytes);
    } else if (file.kind == 'image') {
      body = Text('This image is too large to preview (${sizeText(file.size)}).', key: const ValueKey('files too large'));
    } else {
      body = Text('This file cannot be previewed (${sizeText(file.size)}).', key: const ValueKey('files binary'));
    }
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
                onPressed: () => setState(() {
                  _file = null;
                  _fileProblem = null;
                }),
              ),
              Expanded(
                child: Text(visibleText(file.name), key: const ValueKey('files name'), overflow: TextOverflow.ellipsis),
              ),
              ..._actions(context, file.path),
              if (_fileProblem == null) Text(sizeText(file.size), style: muted),
            ],
          ),
        ),
        const Divider(height: 1),
        if (file.truncated && bytes != null && file.kind == 'text')
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(
              'Showing the first ${sizeText(bytes.length)} of ${sizeText(file.size)}.',
              key: const ValueKey('files truncated'),
              style: muted,
            ),
          ),
        Expanded(
          child: lines == null
              ? SingleChildScrollView(padding: const EdgeInsets.all(16), child: body)
              : ListView.builder(
                  key: const ValueKey('files text'),
                  padding: const EdgeInsets.all(16),
                  itemCount: lines.length,
                  itemBuilder: (context, at) => SelectableText(
                    visibleText(lines![at]),
                    key: ValueKey('files line $at'),
                    style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12),
                  ),
                ),
        ),
      ],
    );
  }

  /// UTF-8 [bytes] as text, a character cut off at the end left out and anything that is not UTF-8 shown as U+FFFD.
  static String _decode(List<int> bytes) {
    var end = bytes.length;
    // A cut can leave the lead of a character and part of its tail: drop at most three bytes.
    for (var back = 1; back <= 3 && back <= end; back++) {
      final byte = bytes[end - back];
      if (byte & 0xC0 == 0x80) continue;
      final need = byte >= 0xF0
          ? 4
          : byte >= 0xE0
          ? 3
          : byte >= 0xC0
          ? 2
          : 1;
      if (need > back) end -= back;
      break;
    }
    return utf8.decode(bytes.sublist(0, end), allowMalformed: true);
  }
}

/// The width and height a PNG, GIF, JPEG or WebP declares in its header, read from the bytes with nothing decoded, or null for bytes
/// that are none of them or are cut before the size. Done here, not by the engine, so it is the same in every build and costs nothing.
({int width, int height})? declaredImageSize(Uint8List b) {
  int be16(int at) => (b[at] << 8) | b[at + 1];
  int le16(int at) => b[at] | (b[at + 1] << 8);
  int be32(int at) => (b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3];
  bool starts(List<int> text, [int at = 0]) {
    if (b.length < at + text.length) return false;
    for (var i = 0; i < text.length; i++) {
      if (b[at + i] != text[i]) return false;
    }
    return true;
  }

  // PNG: the header chunk follows the signature.
  if (starts(const [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) {
    if (b.length < 24 || !starts('IHDR'.codeUnits, 12)) return null;
    return (width: be32(16), height: be32(20));
  }
  if (starts('GIF87a'.codeUnits) || starts('GIF89a'.codeUnits)) {
    return b.length < 10 ? null : (width: le16(6), height: le16(8));
  }
  if (starts(const [0xff, 0xd8])) {
    var at = 2;
    while (at + 4 <= b.length) {
      if (b[at] != 0xff) return null;
      final marker = b[at + 1];
      if (marker == 0xff) {
        at++;
        continue;
      }
      if (marker == 0xd9 || marker == 0xda) return null;
      if (marker == 0x01 || (marker >= 0xd0 && marker <= 0xd8)) {
        at += 2;
        continue;
      }
      final isFrame = marker >= 0xc0 && marker <= 0xcf && marker != 0xc4 && marker != 0xc8 && marker != 0xcc;
      if (isFrame) {
        return at + 9 > b.length ? null : (width: be16(at + 7), height: be16(at + 5));
      }
      at += 2 + be16(at + 2);
    }
    return null;
  }
  if (starts('RIFF'.codeUnits) && starts('WEBP'.codeUnits, 8)) {
    if (starts('VP8 '.codeUnits, 12)) {
      if (b.length < 30 || !starts(const [0x9d, 0x01, 0x2a], 23)) return null;
      return (width: le16(26) & 0x3fff, height: le16(28) & 0x3fff);
    }
    if (starts('VP8L'.codeUnits, 12)) {
      if (b.length < 25 || b[20] != 0x2f) return null;
      final bits = b[21] | (b[22] << 8) | (b[23] << 16) | (b[24] << 24);
      return (width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1);
    }
    if (starts('VP8X'.codeUnits, 12)) {
      if (b.length < 30) return null;
      return (width: (b[24] | (b[25] << 8) | (b[26] << 16)) + 1, height: (b[27] | (b[28] << 8) | (b[29] << 16)) + 1);
    }
  }
  return null;
}

/// An image drawn only once its header says it is small enough: a file of a few bytes can declare dimensions that take gigabytes to
/// decode. It reads the declared size, refuses past [imagePixels], and decodes no larger than [imageSide] on its long side.
class ImagePreview extends StatelessWidget {
  const ImagePreview({super.key, required this.bytes});

  final Uint8List bytes;

  @override
  Widget build(BuildContext context) {
    final size = declaredImageSize(bytes);
    if (size == null || size.width < 1 || size.height < 1) {
      return const Text('This image could not be drawn: its header does not say how large it is.', key: ValueKey('files image failed'));
    }
    if (size.width * size.height > imagePixels) {
      return Text(
        'This image is too large to draw (${size.width} by ${size.height} pixels): decoding it could use more memory than this computer has.',
        key: const ValueKey('files too large'),
      );
    }
    final wide = size.width >= size.height;
    return Image.memory(
      bytes,
      key: const ValueKey('files image'),
      fit: BoxFit.contain,
      cacheWidth: wide && size.width > imageSide ? imageSide : null,
      cacheHeight: !wide && size.height > imageSide ? imageSide : null,
      errorBuilder: (_, _, _) => const Text('This image could not be drawn.'),
    );
  }
}
