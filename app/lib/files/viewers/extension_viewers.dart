import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../l10n/app_text.dart';
import '../../store/links.dart' as links;
import '../../surfaces/a2ui/limits.dart';
import '../../surfaces/a2ui/model.dart';
import '../../surfaces/a2ui/view.dart';
import 'file_sample.dart';
import 'file_viewer.dart';
import 'open_document_stub.dart' if (dart.library.io) 'open_document_io.dart' as platform;

/// Opens a page a viewer drew in the person's browser, and answers whether a browser took it. Tests replace it.
Future<bool> Function(String document, String name) openViewerDocument = platform.openDocument;

/// What a viewer from an extension answers with.
enum ViewerShows { surface, page }

/// A call to the host: the method and its parameters, answered with what the host's result held.
typedef ViewerCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// One viewer the host lists (`file_viewers/list`): what its provider says of it.
final class ExtensionViewerInfo {
  const ExtensionViewerInfo({
    required this.id,
    required this.title,
    required this.shows,
    required this.origin,
    this.help,
    this.mimeTypes = const {},
    this.extensions = const {},
    this.priority = 0,
    this.opensSections = false,
  });

  final String id;
  final String title;
  final ViewerShows shows;

  /// The name of the entry or extension that loads the provider.
  final String origin;
  final String? help;
  final Set<String> mimeTypes;
  final Set<String> extensions;
  final int priority;
  final bool opensSections;

  static ExtensionViewerInfo? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    final shows = switch (raw['shows']) {
      'surface' => ViewerShows.surface,
      'page' => ViewerShows.page,
      _ => null,
    };
    if (shows == null) return null;
    Set<String> words(Object? value) => {
      if (value is List)
        for (final each in value)
          if (each is String && each.isNotEmpty) each,
    };
    final mimeTypes = words(raw['mime_types']);
    final extensions = words(raw['extensions']);
    // A viewer that names no type shows nothing.
    if (mimeTypes.isEmpty && extensions.isEmpty) return null;
    return ExtensionViewerInfo(
      id: raw['id'] as String,
      title: raw['title'] as String,
      shows: shows,
      origin: raw['origin'] is String ? raw['origin'] as String : '',
      help: raw['help'] is String && (raw['help'] as String).isNotEmpty ? raw['help'] as String : null,
      mimeTypes: mimeTypes,
      extensions: extensions,
      priority: raw['priority'] is int ? raw['priority'] as int : 0,
      opensSections: raw['opens_sections'] == true,
    );
  }
}

/// What `file_viewers/render` answered.
sealed class ViewerDrawing {
  const ViewerDrawing();
}

/// The A2UI messages of a surface, which the app draws with its own widgets.
final class SurfaceDrawing extends ViewerDrawing {
  const SurfaceDrawing(this.messages);
  final List<Object?> messages;
}

/// A sealed HTML page, which opens in the person's browser.
final class PageDrawing extends ViewerDrawing {
  const PageDrawing(this.document);
  final String document;
}

/// A viewer an extension offers. The host runs the extension and reads the file; this asks it to draw one file and shows the answer,
/// so an extension's code never runs in the app. The path goes to the host only to name the file it reads: the extension is given
/// the bytes, never the path.
class ExtensionViewer extends FileViewer {
  const ExtensionViewer(this.info, this._call);

  /// What the id of an extension's viewer starts with, so that it never takes the place of one of the app's own.
  static const String idPrefix = 'ext.';

  final ExtensionViewerInfo info;
  final ViewerCall _call;

  @override
  String get id => '$idPrefix${info.id}';
  @override
  String get labelKey => 'files.viewer.extension';
  @override
  String labelFor(BuildContext context) => info.title;
  @override
  Set<String> get mimeTypes => info.mimeTypes;
  @override
  Set<String> get extensions => info.extensions;
  @override
  int get priority => info.priority;
  @override
  bool get opensSections => info.opensSections;

  /// The host reads the file itself, so whatever it sent the app of it, the viewer can be asked.
  @override
  bool canShow(FileSample sample) => true;

  /// Asks the host to draw [request]'s file with this viewer.
  Future<ViewerDrawing> draw(FileViewRequest request) async {
    final answer = await _call('file_viewers/render', {
      'id': info.id,
      'path': request.sample.path,
      if (request.section != null && request.section!.isNotEmpty) 'section': request.section,
    });
    if (answer is Map && answer['shows'] == 'surface' && answer['messages'] is List) {
      return SurfaceDrawing((answer['messages'] as List).cast<Object?>());
    }
    if (answer is Map && answer['shows'] == 'page' && answer['document'] is String) {
      return PageDrawing(answer['document'] as String);
    }
    throw const FormatException('the host answered no drawing');
  }

  @override
  Widget build(BuildContext context, FileViewRequest request) => ExtensionViewerPage(
    key: ValueKey('extension viewer ${info.id} ${request.sample.path} ${request.section ?? ''}'),
    viewer: this,
    request: request,
  );
}

/// An extension's viewer, drawn: asks the host for the file's drawing when it is built, and shows a surface, a notice with the
/// button that opens a page, or why it could not.
class ExtensionViewerPage extends StatefulWidget {
  const ExtensionViewerPage({super.key, required this.viewer, required this.request});

  final ExtensionViewer viewer;
  final FileViewRequest request;

  @override
  State<ExtensionViewerPage> createState() => _ExtensionViewerPageState();
}

class _ExtensionViewerPageState extends State<ExtensionViewerPage> {
  A2uiProcessor? _processor;
  A2uiSurface? _surface;
  String? _document;
  String? _problem;
  bool? _took;
  var _loading = true;
  var _gone = false;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    _gone = true;
    _processor?.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    ViewerDrawing? drawing;
    String? failed;
    try {
      drawing = await widget.viewer.draw(widget.request);
    } on Object catch (error) {
      failed = '$error';
    }
    if (_gone) return;
    A2uiProcessor? processor;
    A2uiSurface? surface;
    String? page;
    switch (drawing) {
      case SurfaceDrawing(:final messages):
        processor = A2uiProcessor(limits: const A2uiLimits());
        processor.receive(messages);
        surface = processor.surfaces.values.where((each) => each.hasRoot).firstOrNull;
        if (surface == null) failed = processor.problems.isEmpty ? '' : '${processor.problems.first}';
      case PageDrawing(:final document):
        page = document;
      case null:
        break;
    }
    setState(() {
      _loading = false;
      _processor = processor;
      _surface = surface;
      _document = page;
      _problem = failed;
    });
  }

  Future<void> _open() async {
    final document = _document;
    if (document == null) return;
    var took = false;
    try {
      took = await openViewerDocument(document, widget.request.sample.name);
    } on Object {
      took = false;
    }
    if (!_gone) setState(() => _took = took);
  }

  @override
  Widget build(BuildContext context) {
    final name = widget.viewer.info.title;
    if (_loading) {
      return const Center(child: CircularProgressIndicator(key: ValueKey('extension viewer loading')));
    }
    final surface = _surface;
    if (surface != null) {
      return A2uiSurfaceView(
        key: ValueKey('extension viewer surface ${widget.viewer.info.id}'),
        surface: surface,
        onOpenUrl: links.openLink,
        words: A2uiWords(
          problems: context.t('a2ui.problems'),
          waiting: context.t('a2ui.waiting'),
          moreRows: (count) => context.t('a2ui.more_rows', {'n': '$count'}),
        ),
      );
    }
    if (_document != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(context.t('files.viewer.page', {'name': name}), key: const ValueKey('extension viewer page'), textAlign: TextAlign.center),
              const SizedBox(height: 12),
              FilledButton.icon(
                key: const ValueKey('extension viewer open page'),
                onPressed: _open,
                icon: const Icon(Icons.open_in_new),
                label: Text(context.t('files.viewer.open_page')),
              ),
              if (_took == false)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    context.t('files.viewer.page_failed'),
                    key: const ValueKey('extension viewer page failed'),
                    style: TextStyle(color: Theme.of(context).colorScheme.error),
                  ),
                ),
            ],
          ),
        ),
      );
    }
    final why = _problem ?? '';
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Text(
          why.isEmpty ? context.t('files.viewer.nothing', {'name': name}) : context.t('files.viewer.failed', {'name': name, 'why': why}),
          key: const ValueKey('extension viewer failed'),
          textAlign: TextAlign.center,
        ),
      ),
    );
  }
}

/// The viewers the host's extensions offer, in a [ViewerRegistry] beside the app's own: asks the host which there are and registers
/// each, and takes away the ones it no longer lists. The app knows none by name.
final class ExtensionViewers {
  ExtensionViewers(this._call);

  final ViewerCall _call;
  ViewerRegistry? _registry;
  List<ExtensionViewerInfo> _listed = const [];
  final Set<String> _registered = {};

  /// The viewers the host lists now.
  List<ExtensionViewerInfo> get listed => _listed;

  /// Registers the host's viewers in [registry] from now on, and the ones it already listed.
  void attach(ViewerRegistry registry) {
    _registry = registry;
    _apply();
  }

  /// Takes the host's answer to `file_viewers/list`.
  void adopt(Object? answer) {
    final raw = answer is Map ? answer['viewers'] : null;
    _listed = [if (raw is List) for (final each in raw) ?ExtensionViewerInfo.read(each)];
    _apply();
  }

  /// Asks the host for its viewers again; a host that cannot answer lists none.
  Future<void> load() async {
    try {
      adopt(await _call('file_viewers/list', const {}));
    } on Object {
      adopt(null);
    }
  }

  void _apply() {
    final registry = _registry;
    if (registry == null) return;
    for (final id in _registered) {
      registry.unregister(id);
    }
    _registered.clear();
    for (final info in _listed) {
      final viewer = ExtensionViewer(info, _call);
      registry.register(viewer);
      _registered.add(viewer.id);
    }
  }
}
