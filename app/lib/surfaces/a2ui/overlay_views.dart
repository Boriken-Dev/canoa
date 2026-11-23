/// The views an overlay ships: `catalogs/NAME.json` (components that rename and preset ones Canoa draws) and `surfaces/NAME.json`
/// (`{"title", "target", "messages": [A2UI messages]}`, a fixed view with its data). An overlay chooses structure; the colours are
/// the look's, and a view that names a component or a catalog Canoa does not have is refused, with why, and draws nothing.
library;

import 'dart:convert';

import '../../store/overlay_contents.dart' show ViewFile;
import 'catalog.dart';
import 'limits.dart';
import 'model.dart';
import 'views.dart' show A2uiTarget;

/// One view of an overlay.
final class OverlayView {
  const OverlayView({required this.name, required this.title, required this.target, required this.messages, this.problems = const []});

  /// The file's name without its folder and extension.
  final String name;
  final String title;
  final A2uiTarget target;
  final List<Object?> messages;

  /// Why it cannot be drawn; a view with any is listed with them and never opened.
  final List<String> problems;

  bool get drawable => problems.isEmpty;
}

/// Everything an overlay's files say about its views.
final class OverlayViews {
  const OverlayViews({this.views = const [], this.catalogs = const [], this.problems = const []});

  final List<OverlayView> views;
  final List<A2uiCatalog> catalogs;

  /// What was wrong with files that are not a view or a catalog.
  final List<String> problems;

  bool get isEmpty => views.isEmpty && catalogs.isEmpty && problems.isEmpty;
}

String _nameOf(String path) {
  final file = path.split('/').last;
  return file.endsWith('.json') ? file.substring(0, file.length - 5) : file;
}

/// Reads [files] (`catalogs/*.json` and `surfaces/*.json` of an overlay, as the host read them), each view tried once against the
/// catalogs the overlay brings so that what cannot be drawn is said before anyone opens it.
OverlayViews readOverlayViews(List<ViewFile> files, {A2uiLimits limits = const A2uiLimits()}) {
  final problems = <String>[];
  final catalogs = <A2uiCatalog>[];
  Object? decode(ViewFile file) {
    try {
      return jsonDecode(file.text);
    } on FormatException {
      problems.add('${file.path}: not JSON');
      return null;
    }
  }

  for (final file in files.where((each) => each.path.startsWith('catalogs/'))) {
    final document = decode(file);
    if (document == null) continue;
    final read = A2uiCatalog.fromOverlay(document);
    if (read.catalog != null) {
      catalogs.add(read.catalog!);
    } else {
      problems.addAll([for (final each in read.problems) '${file.path}: $each']);
    }
  }
  final views = <OverlayView>[];
  for (final file in files.where((each) => each.path.startsWith('surfaces/'))) {
    final document = decode(file);
    if (document == null) continue;
    final name = _nameOf(file.path);
    if (document is! Map || document['messages'] is! List) {
      views.add(OverlayView(name: name, title: name, target: A2uiTarget.sidePanel, messages: const [], problems: const ['A view is {"messages": [...]}']));
      continue;
    }
    final messages = document['messages'] as List<Object?>;
    final found = <String>[];
    final trial = A2uiProcessor(limits: limits, catalogs: A2uiCatalogs(catalogs));
    trial.receive(messages);
    found.addAll([for (final each in trial.problems) '$each']);
    for (final surface in trial.surfaces.values) {
      found.addAll([for (final each in surface.problems) '$each']);
      if (!surface.hasRoot) found.add('${surface.id}: no component has the id root');
    }
    if (trial.surfaces.isEmpty && found.isEmpty) found.add('It creates no surface');
    trial.dispose();
    views.add(OverlayView(
      name: name,
      title: document['title'] is String && (document['title'] as String).trim().isNotEmpty ? (document['title'] as String).trim() : name,
      target: A2uiTarget.read(document['target'] is String ? document['target'] as String : null),
      messages: messages,
      problems: found,
    ));
  }
  return OverlayViews(views: views, catalogs: catalogs, problems: problems);
}

/// [messages] with each surface id made the overlay's own, so two overlays' views never meet.
List<Object?> scopedMessages(String overlay, List<Object?> messages) => [
  for (final message in messages)
    if (message is Map)
      {
        for (final entry in message.entries)
          '${entry.key}': entry.key != 'version' && entry.value is Map && (entry.value as Map)['surfaceId'] is String
              ? {...(entry.value as Map).map((key, each) => MapEntry('$key', each)), 'surfaceId': '$a2uiOverlayPrefix$overlay:${(entry.value as Map)['surfaceId']}'}
              : entry.value,
      }
    else
      message,
];

/// What every surface id of an overlay's view starts with.
const a2uiOverlayPrefix = 'overlay:';

/// Draws [view] of the overlay named [overlay]; answers why it could not, empty when it did.
typedef OpenOverlayView = List<String> Function(String overlay, OverlayView view, OverlayViews brought);
