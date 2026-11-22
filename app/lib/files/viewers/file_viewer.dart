import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';

import '../../shell/layout_state.dart' show LayoutBackend, MemoryLayoutBackend;
import 'file_sample.dart';

/// What a viewer is asked to draw: the file, its MIME type and, for a document with sections, the one to open at.
final class FileViewRequest {
  const FileViewRequest({required this.sample, required this.type, this.section});

  final FileSample sample;

  /// The MIME type [FileTypes.typeOf] gave the file.
  final String type;

  /// The heading to focus, by its anchor or its title.
  final String? section;
}

/// One way to show a file. The Files surface asks a [ViewerRegistry] which viewers can show the file it opened, draws the one the
/// person's choice or the registry's order names, and offers the others as a switch. A viewer draws only; it never edits.
abstract class FileViewer {
  const FileViewer();

  /// Unique among the registry's viewers; the person's choice per type is kept by it.
  String get id;

  /// The text key of the name the viewer's switch shows.
  String get labelKey;

  /// The MIME types it shows: exact (`application/json`) or by family (`image/*`).
  Set<String> get mimeTypes => const {};

  /// The extensions, without the dot, it shows whatever the type says.
  Set<String> get extensions => const {};

  /// Which of two viewers that can show a file is tried first: the higher.
  int get priority => 0;

  /// Whether it only shows what no other viewer can, and is left out of the switch.
  bool get fallback => false;

  /// Whether the viewer opens a document at a section ([FileViewRequest.section]).
  bool get opensSections => false;

  /// Whether the data the host read is the kind this viewer draws: text for a text viewer, the bytes of an image for an image
  /// viewer. A viewer that cannot read the file never matches it, however its name or type reads.
  bool canShow(FileSample sample);

  /// Whether this viewer can show [sample], whose type is [type].
  bool matches(FileSample sample, String type) => canShow(sample) && (_typeMatches(type) || extensions.contains(sample.extension));

  bool _typeMatches(String type) {
    for (final mime in mimeTypes) {
      if (mime == type || (mime.endsWith('/*') && type.startsWith(mime.substring(0, mime.length - 1)))) return true;
    }
    return false;
  }

  /// Whether it is the one to open [sample] with when the person has not chosen: a viewer that suits only some of the files it can
  /// show (a Markdown file with sections) says so here, and the next one is tried when it is false.
  bool isDefaultFor(FileViewRequest request) => true;

  /// The viewer's page, which fills the room it is given and scrolls itself.
  Widget build(BuildContext context, FileViewRequest request);
}

/// The viewer the person chose for each type of file, kept as one JSON document (`{"application/json": "text"}`) where the
/// [LayoutBackend] puts it. Nothing is kept for a type the person never chose.
class ViewerChoices extends ChangeNotifier {
  ViewerChoices({LayoutBackend? backend}) : _backend = backend ?? MemoryLayoutBackend();

  final LayoutBackend _backend;
  final Map<String, String> _byType = {};
  Future<void>? _loading;
  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  /// The id of the viewer chosen for [type], or null.
  String? chosen(String type) => _byType[type];

  /// Reads the choices once; a document that cannot be read leaves them empty.
  Future<void> load() => _loading ??= _read();

  Future<void> _read() async {
    try {
      final raw = await _backend.read();
      if (raw == null) return;
      final json = jsonDecode(raw);
      if (json is Map) {
        for (final MapEntry(:key, :value) in json.entries) {
          if (key is String && value is String) _byType.putIfAbsent(key, () => value);
        }
        if (!_disposed) notifyListeners();
      }
    } on Object {
      // A choice that cannot be read is not made: the registry's order shows the file.
    }
  }

  /// Makes [viewerId] the viewer of files of [type], or none of them for null.
  Future<void> choose(String type, String? viewerId) async {
    await load();
    if (viewerId == null) {
      _byType.remove(type);
    } else {
      _byType[type] = viewerId;
    }
    if (!_disposed) notifyListeners();
    try {
      await _backend.write(jsonEncode(_byType));
    } on Object {
      // Kept for this run; the next run reads the old choice.
    }
  }
}

/// The viewers a file can be shown with, and which one it is shown with. Built-in viewers and ones an extension adds are the same
/// kind of object: a viewer is added with [register] and left with [unregister], and the open file redraws.
class ViewerRegistry extends ChangeNotifier {
  ViewerRegistry({FileTypes? types, ViewerChoices? choices, Iterable<FileViewer> viewers = const []})
    : types = types ?? FileTypes(),
      choices = choices ?? ViewerChoices() {
    // A choice read or made redraws what is open.
    this.choices.addListener(notifyListeners);
    viewers.forEach(register);
  }

  /// How a file's MIME type is worked out; extensions add their own extensions and sniffers to it.
  final FileTypes types;

  /// The person's choice of viewer per type.
  final ViewerChoices choices;

  final List<FileViewer> _viewers = [];

  /// The registered viewers, in the order they were added.
  List<FileViewer> get viewers => List.unmodifiable(_viewers);

  /// Adds [viewer], replacing the one with its id.
  void register(FileViewer viewer) {
    final at = _viewers.indexWhere((each) => each.id == viewer.id);
    if (at == -1) {
      _viewers.add(viewer);
    } else {
      _viewers[at] = viewer;
    }
    notifyListeners();
  }

  /// Removes the viewer with [id], and says whether there was one.
  bool unregister(String id) {
    final before = _viewers.length;
    _viewers.removeWhere((each) => each.id == id);
    final removed = _viewers.length != before;
    if (removed) notifyListeners();
    return removed;
  }

  /// The type of [sample].
  String typeOf(FileSample sample) => types.typeOf(sample);

  /// The viewers that can show [sample], the one tried first first: a higher priority before a lower, and the earlier registered
  /// before the later. A viewer that is only a fallback is listed only when no other can show the file.
  List<FileViewer> matching(FileSample sample, {String? type}) {
    final kind = type ?? typeOf(sample);
    final found = [
      for (final each in _viewers)
        if (each.matches(sample, kind)) each,
    ];
    final order = {for (var i = 0; i < _viewers.length; i++) _viewers[i].id: i};
    int byOrder(FileViewer a, FileViewer b) => b.priority != a.priority ? b.priority.compareTo(a.priority) : order[a.id]!.compareTo(order[b.id]!);
    final first = found.where((each) => !each.fallback).toList()..sort(byOrder);
    return first.isNotEmpty ? first : (found..sort(byOrder));
  }

  /// The viewer that shows [sample]: the person's choice for its type when that viewer can show it, a viewer that opens the
  /// [section] asked for, then the first that finds the file one it suits. Null only when nothing is registered that can show it.
  FileViewer? choose(FileSample sample, {String? type, String? section}) {
    final kind = type ?? typeOf(sample);
    final all = matching(sample, type: kind);
    if (all.isEmpty) return null;
    final request = FileViewRequest(sample: sample, type: kind, section: section);
    if (section != null && section.isNotEmpty) {
      final opens = all.where((each) => each.opensSections).firstOrNull;
      if (opens != null) return opens;
    }
    final saved = choices.chosen(kind);
    final byChoice = saved == null ? null : all.where((each) => each.id == saved).firstOrNull;
    return byChoice ?? all.where((each) => each.isDefaultFor(request)).firstOrNull ?? all.first;
  }

  @override
  void dispose() {
    choices.dispose();
    super.dispose();
  }
}
