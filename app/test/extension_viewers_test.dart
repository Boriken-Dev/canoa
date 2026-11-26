// Viewers an extension registers for the Files surface: the host lists them (`file_viewers/list`), the registry takes them in beside
// the app's own, and a file is drawn by asking the host (`file_viewers/render`), which answers an A2UI surface the app draws or a page
// that opens in the browser. Tried against a stand-in for the host, which plays an extension's provider.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/files/viewers/extension_viewers.dart';
import 'package:canoa/files/viewers/file_sample.dart';
import 'package:canoa/files/viewers/file_viewer.dart';
import 'package:canoa/files/viewers/standard_viewers.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

/// A host's disk in memory.
final class _Disk implements WorkspaceFiles {
  _Disk(this.files);

  final Map<String, Uint8List> files;
  static const root = '/work';

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async => HostFolderListing(
    path: root,
    parent: null,
    entries: [for (final name in this.files.keys) HostFolderEntry(name: name, path: '$root/$name', isFolder: false)],
    more: false,
  );

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    final name = path.split('/').last;
    final all = files[name]!;
    final text = !all.contains(0);
    return HostFileRead(path: path, name: name, size: all.length, kind: text ? 'text' : 'binary', truncated: false, bytes: text ? all : null);
  }
}

Uint8List _text(String text) => Uint8List.fromList(utf8.encode(text));

Finder _key(String name) => find.byKey(ValueKey(name));

/// What the host lists for the stand-in extension: the rows `file_viewers/list` answers.
Map<String, Object?> _row(String id, String title, {String shows = 'surface', List<String> extensions = const [], List<String> mimeTypes = const [], int priority = 0}) => {
  'id': id,
  'title': title,
  'help': null,
  'mime_types': mimeTypes,
  'extensions': extensions,
  'priority': priority,
  'opens_sections': false,
  'shows': shows,
  'origin': 'viewers',
};

/// A host with an extension that shows `.stamp` files as a surface and `.dump` files as a page; every call is recorded.
final class _StandInHost {
  _StandInHost(this.files, {List<Map<String, Object?>>? rows}) : rows = rows ?? [_row('stamp', 'Stamp', extensions: ['stamp'], priority: 20), _row('dump', 'Dump', shows: 'page', extensions: ['dump'])];

  final Map<String, Uint8List> files;
  final List<Map<String, Object?>> rows;
  final List<(String, Map<String, Object?>)> calls = [];
  Object? failure;

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    if (failure != null) throw failure!;
    switch (method) {
      case 'file_viewers/list':
        return {'viewers': rows};
      case 'file_viewers/render':
        final path = params['path']! as String;
        final bytes = files[path.split('/').last]!;
        if (params['id'] == 'dump') {
          return {'id': 'dump', 'shows': 'page', 'document': '<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src \'none\'"><p>${bytes.length} bytes</p>'};
        }
        return {
          'id': params['id'],
          'shows': 'surface',
          'messages': [
            {
              'version': '1.0',
              'createSurface': {
                'surfaceId': 'stamp',
                'components': [
                  {'id': 'root', 'component': 'Text', 'text': 'stamped ${path.split('/').last}: ${utf8.decode(bytes, allowMalformed: true)}'},
                ],
              },
            },
          ],
        };
    }
    throw StateError('unexpected $method');
  }
}

Future<void> _open(WidgetTester tester, Map<String, Uint8List> files, String name, ViewerRegistry registry) async {
  await tester.binding.setSurfaceSize(const Size(560, 640));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: FilesPanel(files: _Disk(files), root: _Disk.root, viewers: registry)),
    ),
  );
  await tester.pumpAndSettle();
  await tester.tap(_key('files entry $name'));
  await tester.pumpAndSettle();
}

void main() {
  group('the listing', () {
    test('a viewer is read from the host\'s row, and one that names no type or no way to show is left out', () {
      final info = ExtensionViewerInfo.read(_row('stamp', 'Stamp', extensions: ['stamp'], mimeTypes: ['text/x-stamp'], priority: 20))!;
      expect(info.id, 'stamp');
      expect(info.title, 'Stamp');
      expect(info.extensions, {'stamp'});
      expect(info.mimeTypes, {'text/x-stamp'});
      expect(info.priority, 20);
      expect(info.shows, ViewerShows.surface);
      expect(info.origin, 'viewers');
      expect(ExtensionViewerInfo.read(_row('a', 'A', shows: 'page', mimeTypes: ['a/b']))!.shows, ViewerShows.page);
      expect(ExtensionViewerInfo.read(_row('a', 'A')), isNull, reason: 'no type');
      expect(ExtensionViewerInfo.read({..._row('a', 'A', extensions: ['a']), 'shows': 'terminal'}), isNull);
      expect(ExtensionViewerInfo.read({'id': 'a'}), isNull);
      expect(ExtensionViewerInfo.read('no'), isNull);
    });

    test('the host\'s viewers join the registry under a name of their own, and leave with the extension', () async {
      final host = _StandInHost({});
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      final builtIn = registry.viewers.map((each) => each.id).toList();
      await viewers.load();
      expect(registry.viewers.map((each) => each.id), [...builtIn, 'ext.stamp', 'ext.dump']);
      expect(host.calls.single, ('file_viewers/list', const <String, Object?>{}));

      // A viewer that says it is `text` is the extension's `text`, not the app's.
      host.rows
        ..clear()
        ..add(_row('text', 'Mine', extensions: ['txt']));
      await viewers.load();
      expect(registry.viewers.map((each) => each.id), [...builtIn, 'ext.text']);

      host.rows.clear();
      await viewers.load();
      expect(registry.viewers.map((each) => each.id), builtIn);
    });

    test('a host that cannot answer lists none, and a registry attached late gets what was listed', () async {
      final host = _StandInHost({})..failure = StateError('no link');
      final viewers = ExtensionViewers(host.call);
      await viewers.load();
      expect(viewers.listed, isEmpty);
      host.failure = null;
      await viewers.load();
      final registry = standardViewerRegistry();
      expect(registry.viewers.any((each) => each.id.startsWith('ext.')), isFalse);
      viewers.attach(registry);
      expect(registry.viewers.where((each) => each.id.startsWith('ext.')), hasLength(2));
    });

    test('a viewer of an extension is chosen for its type before the app\'s own when its priority is higher, and not otherwise', () async {
      final host = _StandInHost({}, rows: [_row('stamp', 'Stamp', extensions: ['stamp'], priority: 20), _row('quiet', 'Quiet', extensions: ['stamp'])]);
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      await viewers.load();
      final sample = FileSample(HostFileRead(path: '/work/a.stamp', name: 'a.stamp', size: 1, kind: 'text', truncated: false, bytes: _text('x')));
      expect(registry.choose(sample)!.id, 'ext.stamp');
      expect(registry.matching(sample).map((each) => each.id), ['ext.stamp', 'ext.quiet', 'text'], reason: 'its own priority, then the order');
      final other = FileSample(HostFileRead(path: '/work/a.txt', name: 'a.txt', size: 1, kind: 'text', truncated: false, bytes: _text('x')));
      expect(registry.choose(other)!.id, 'text');
    });
  });

  group('drawing', () {
    testWidgets('a surface the host drew is shown in the panel, and the host is asked for the file by its path only', (tester) async {
      final files = {'a.stamp': _text('hello'), 'b.txt': _text('plain')};
      final host = _StandInHost(files);
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      await tester.runAsync(viewers.load);
      await _open(tester, files, 'a.stamp', registry);
      await tester.pumpAndSettle();
      expect(find.text('stamped a.stamp: hello'), findsOneWidget);
      final render = host.calls.where((each) => each.$1 == 'file_viewers/render').single;
      expect(render.$2, {'id': 'stamp', 'path': '/work/a.stamp'});
      // The extension's viewer is one of two the file has: a switch names it by its own title.
      expect(_key('viewer switch ext.stamp'), findsOneWidget);
      expect(find.text('Stamp'), findsOneWidget);
      expect(_key('viewer switch text'), findsOneWidget);

      await tester.tap(_key('viewer switch text'));
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget);
      expect(find.text('stamped a.stamp: hello'), findsNothing);
      expect(registry.choices.chosen('application/octet-stream'), isNull);
    });

    testWidgets('with more viewers than a row of buttons holds, the choice is an Open with menu', (tester) async {
      final files = {'a.stamp': _text('hello')};
      final rows = [for (final each in ['one', 'two', 'three', 'four']) _row(each, each.toUpperCase(), extensions: ['stamp'], priority: 20 - each.length)];
      final host = _StandInHost(files, rows: rows);
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      await tester.runAsync(viewers.load);
      await _open(tester, files, 'a.stamp', registry);
      expect(_key('viewer switch'), findsNothing);
      expect(_key('viewer menu'), findsOneWidget);
      expect(find.text('Open with'), findsOneWidget);
      expect(tester.widget<Text>(_key('viewer menu current')).data, 'ONE', reason: 'the first of equal priority is the one shown');
      await tester.tap(_key('viewer menu'));
      await tester.pumpAndSettle();
      expect(_key('viewer menu ext.two'), findsOneWidget);
      expect(_key('viewer menu text'), findsOneWidget);
      await tester.tap(_key('viewer menu text'));
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget);
    });

    testWidgets('a page the host drew opens in the browser from a button, and a build that cannot open one says so', (tester) async {
      final files = {'x.dump': Uint8List.fromList([0, 1, 2, 3])};
      final host = _StandInHost(files);
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      await tester.runAsync(viewers.load);
      final opened = <(String, String)>[];
      var took = true;
      final before = openViewerDocument;
      addTearDown(() => openViewerDocument = before);
      openViewerDocument = (document, name) async {
        opened.add((document, name));
        return took;
      };
      await _open(tester, files, 'x.dump', registry);
      expect(find.text('Dump draws this file as a page, which opens in your browser.'), findsOneWidget);
      expect(opened, isEmpty, reason: 'nothing opens until the person asks');
      await tester.tap(_key('extension viewer open page'));
      await tester.pumpAndSettle();
      expect(opened.single.$2, 'x.dump');
      expect(opened.single.$1, contains('Content-Security-Policy'));
      expect(opened.single.$1, contains('4 bytes'));
      expect(_key('extension viewer page failed'), findsNothing);

      took = false;
      await tester.tap(_key('extension viewer open page'));
      await tester.pumpAndSettle();
      expect(_key('extension viewer page failed'), findsOneWidget);
    });

    testWidgets('a viewer the host cannot draw with says why, in the panel', (tester) async {
      final files = {'a.stamp': _text('hello')};
      final host = _StandInHost(files);
      final registry = standardViewerRegistry();
      final viewers = ExtensionViewers(host.call)..attach(registry);
      await tester.runAsync(viewers.load);
      host.failure = StateError('the viewer broke');
      await _open(tester, files, 'a.stamp', registry);
      expect(_key('extension viewer failed'), findsOneWidget);
      expect(find.textContaining('Stamp could not draw this file'), findsOneWidget);
      expect(find.textContaining('the viewer broke'), findsOneWidget);
    });
  });
}
