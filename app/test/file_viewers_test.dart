// The Files surface's viewers: a registry that picks one by a file's type (the host's MIME type, the extension, a look at the
// first bytes), the person's choice per type kept, and the built-in viewers (text, Markdown sections, image, a JSON/YAML/TOML tree,
// a diff, and the notice for a file nothing can draw).
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/files/viewers/diff_viewer.dart';
import 'package:canoa/files/viewers/file_sample.dart';
import 'package:canoa/files/viewers/file_viewer.dart';
import 'package:canoa/files/viewers/standard_viewers.dart';
import 'package:canoa/files/viewers/structured_viewer.dart';
import 'package:canoa/files/viewers/text_viewer.dart' show TextLines;
import 'package:canoa/highlight/syntax.dart' show languageForInfo;
import 'package:canoa/highlight/syntax_theme.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLook;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/shell/layout_state.dart' show MemoryLayoutBackend;
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A host's disk in memory: each file is its name and what the host would read of it.
final class _Disk implements WorkspaceFiles {
  _Disk(this.files, {this.limit = FilesPanel.previewBytes});

  final Map<String, Uint8List> files;
  final int limit;
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
    final cut = all.length > limit;
    final bytes = cut ? all.sublist(0, limit) : all;
    final image = bytes.length > 4 && bytes[0] == 0x89 && bytes[1] == 0x50;
    final text = !image && !bytes.contains(0);
    return HostFileRead(
      path: path,
      name: name,
      size: all.length,
      kind: image ? 'image' : (text ? 'text' : 'binary'),
      truncated: cut,
      bytes: (text || (image && !cut)) ? bytes : null,
    );
  }
}

Uint8List _text(String text) => Uint8List.fromList(utf8.encode(text));

/// A one-pixel PNG.
final _png = base64Decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==');

const _json = '{"name": "canoa", "version": "0.1.0", "private": true, "scripts": {"test": "flutter test", "build": "flutter build"}, "keywords": ["chat", "agents"], "license": null, "size": 12}';
const _yaml = 'name: canoa\nports:\n  - 80\n  - 443\nnested:\n  deep:\n    ok: true\n';
const _toml = 'title = "canoa"\n[host]\nport = 8080\nnames = ["a", "b"]\n';
const _diff = 'diff --git a/lib/a.dart b/lib/a.dart\nindex 111..222 100644\n--- a/lib/a.dart\n+++ b/lib/a.dart\n@@ -1,4 +1,4 @@\n void main() {\n-  print(1);\n+  print(2);\n }\n';

FileSample _sample(String name, {String? mime, String? text, String kind = 'text', Uint8List? bytes, bool truncated = false}) => FileSample(
  HostFileRead(path: '/work/$name', name: name, size: bytes?.length ?? text?.length ?? 0, kind: kind, truncated: truncated, mime: mime, bytes: bytes ?? (text == null ? null : _text(text))),
);

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _show(WidgetTester tester, Widget child, {Size size = const Size(560, 640), Brightness brightness = Brightness.dark}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(LookId.canoa, brightness),
      home: Scaffold(body: child),
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> _open(WidgetTester tester, Map<String, Uint8List> files, String name, {ViewerRegistry? viewers, Size size = const Size(560, 640), int? limit}) async {
  await _show(tester, FilesPanel(files: _Disk(files, limit: limit ?? FilesPanel.previewBytes), root: _Disk.root, viewers: viewers), size: size);
  await tester.tap(_key('files entry $name'));
  await tester.pumpAndSettle();
}

/// A viewer an extension might add: it shows files of its own type, or of its own extension.
final class _Stamp extends FileViewer {
  const _Stamp({this.mime = 'application/x-stamp', this.ext = 'stamp'});
  final String mime;
  final String ext;

  @override
  String get id => 'stamp';
  @override
  String get labelKey => 'files.viewer.diff';
  @override
  Set<String> get mimeTypes => {mime};
  @override
  Set<String> get extensions => {ext};
  @override
  int get priority => 50;
  @override
  bool canShow(FileSample sample) => sample.isText;
  @override
  Widget build(BuildContext context, FileViewRequest request) => Text('stamped ${request.sample.name}', key: const ValueKey('stamp page'));
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/file_viewers_test.dart')));

  group('the type of a file', () {
    final types = FileTypes();
    test('is the host\'s own word when it is specific, then the extension, then the first bytes, then what the host sorted it as', () {
      expect(types.typeOf(_sample('x.bin', mime: 'application/json; charset=utf-8', text: 'hi')), 'application/json');
      expect(types.typeOf(_sample('notes.MD', mime: 'text/plain', text: 'hi')), 'text/markdown', reason: 'text/plain says nothing a name does not say better');
      expect(types.typeOf(_sample('data', text: '{"a": [1, 2]}')), 'application/json', reason: 'JSON by its first bytes');
      expect(types.typeOf(_sample('data', text: '{not json')), FileTypes.plain);
      expect(types.typeOf(_sample('changes', text: _diff)), 'text/x-diff');
      expect(types.typeOf(_sample('pic', kind: 'image', bytes: _png)), 'image/png');
      expect(types.typeOf(_sample('blob', kind: 'binary')), FileTypes.unknown);
      expect(types.typeOf(_sample('.gitignore', text: 'a')), FileTypes.plain, reason: 'a leading dot is part of the name, not an extension');
    });

    test('learns an extension and a sniffer an extension brings', () {
      final own = FileTypes()
        ..addExtension('.stamp', 'application/x-stamp')
        ..addSniffer((sample) => sample.text?.startsWith('STAMP') == true ? 'application/x-stamp' : null);
      expect(own.typeOf(_sample('a.stamp', text: 'x')), 'application/x-stamp');
      expect(own.typeOf(_sample('a', text: 'STAMP 1')), 'application/x-stamp');
    });
  });

  group('the registry', () {
    test('lists the viewers that can show a file, a higher priority first, and a fallback only when nothing else can', () {
      final registry = standardViewerRegistry();
      List<String> ids(FileSample sample) => [for (final each in registry.matching(sample)) each.id];
      expect(ids(_sample('a.txt', text: 'hi')), ['text']);
      expect(ids(_sample('a.md', text: '# A')), ['markdown', 'text']);
      expect(ids(_sample('a.json', text: '{}')), ['tree', 'text']);
      expect(ids(_sample('a.patch', text: _diff)), ['diff', 'text']);
      expect(ids(_sample('a.png', kind: 'image', bytes: _png)), ['image']);
      expect(ids(_sample('a.bin', kind: 'binary')), ['info']);
      expect(ids(_sample('big.png', kind: 'image', truncated: true)), ['info'], reason: 'an image too large to read is no image to draw');
      expect(ids(_sample('cut.json', text: '{"a": [', truncated: true)), ['text'], reason: 'a document the preview cut is not a document');
    });

    test('chooses the first viewer the file suits, then the person\'s choice for the type', () async {
      final registry = standardViewerRegistry();
      final short = _sample('a.md', text: '# A\n\ntext\n');
      final long = _sample('b.md', text: '---\nk: v\n---\n# A\n');
      expect(registry.choose(short)!.id, 'text', reason: 'a short Markdown file reads plain');
      expect(registry.choose(long)!.id, 'markdown');
      expect(registry.choose(short, section: 'a')!.id, 'markdown', reason: 'a section asked for opens the sectioned view');
      await registry.choices.choose('text/markdown', 'text');
      expect(registry.choose(long)!.id, 'text', reason: 'the choice is per type');
      expect(registry.choose(_sample('a.json', text: '{}'))!.id, 'tree');
      await registry.choices.choose('text/markdown', 'image');
      expect(registry.choose(long)!.id, 'markdown', reason: 'a choice of a viewer that cannot show the file is not followed');
    });

    test('keeps the choices where the backend puts them, and reads them back', () async {
      final backend = MemoryLayoutBackend();
      await ViewerChoices(backend: backend).choose('application/json', 'text');
      final again = ViewerChoices(backend: backend);
      expect(again.chosen('application/json'), isNull, reason: 'not read yet');
      await again.load();
      expect(again.chosen('application/json'), 'text');
      await again.choose('application/json', null);
      final last = ViewerChoices(backend: backend);
      await last.load();
      expect(last.chosen('application/json'), isNull);
      final broken = ViewerChoices(backend: MemoryLayoutBackend()..kept = 'not json');
      await broken.load();
      expect(broken.chosen('application/json'), isNull, reason: 'a document that cannot be read is no choice');
    });

    test('takes a viewer another party registers by type or by extension, and drops it again', () {
      final registry = standardViewerRegistry()..types.addExtension('stamp', 'application/x-stamp');
      final sample = _sample('a.stamp', text: 'x');
      expect(registry.choose(sample)!.id, 'text');
      var changes = 0;
      registry.addListener(() => changes++);
      registry.register(const _Stamp());
      expect(registry.choose(sample)!.id, 'stamp');
      expect(registry.choose(_sample('a.other', text: 'x'))!.id, 'text', reason: 'only its own types');
      registry.register(const _Stamp(mime: 'text/x-nothing', ext: 'zz'));
      expect(registry.viewers.where((each) => each.id == 'stamp'), hasLength(1), reason: 'the same id replaces');
      expect(registry.choose(sample)!.id, 'text');
      expect(registry.unregister('stamp'), isTrue);
      expect(registry.unregister('stamp'), isFalse);
      expect(changes, 3);
    });
  });

  group('the Files panel', () {
    testWidgets('a file of each type opens in its viewer', (tester) async {
      final disk = {
        'a.txt': _text('plain words\n'),
        'b.md': _text('---\nstatus: ok\n---\n# One\n\ntext\n'),
        'c.png': Uint8List.fromList(_png),
        'd.json': _text(_json),
        'e.yaml': _text(_yaml),
        'f.toml': _text(_toml),
        'g.patch': _text(_diff),
        'h.bin': Uint8List.fromList([0, 1, 2, 3]),
      };
      await _open(tester, disk, 'a.txt');
      expect(_key('files text'), findsOneWidget);
      expect(_key('viewer switch'), findsNothing, reason: 'one viewer is no choice');

      await _open(tester, disk, 'b.md');
      expect(_key('markdown section viewer'), findsOneWidget);
      expect(_key('viewer switch markdown'), findsOneWidget);

      await _open(tester, disk, 'c.png');
      expect(_key('files image'), findsOneWidget);

      await _open(tester, disk, 'd.json');
      expect(_key('tree rows'), findsOneWidget);
      expect(find.text('"canoa"'), findsOneWidget);

      await _open(tester, disk, 'e.yaml');
      expect(_key('tree rows'), findsOneWidget);
      expect(_key('tree value ports/1'), findsOneWidget);

      await _open(tester, disk, 'f.toml');
      expect(_key('tree value host/port'), findsOneWidget);

      await _open(tester, disk, 'g.patch');
      expect(_key('diff lines'), findsOneWidget);
      expect(_key('diff added 7'), findsOneWidget);
      expect(_key('diff removed 6'), findsOneWidget);
      expect(_key('diff hunk 4'), findsOneWidget);

      await _open(tester, disk, 'h.bin');
      expect(_key('files binary'), findsOneWidget);
      expect(find.text('This file cannot be previewed (4 bytes).'), findsOneWidget);
      expect(find.text('Type: application/octet-stream'), findsOneWidget);
    });

    testWidgets('a type nobody has a viewer for is shown as text, and one by its first bytes as its own', (tester) async {
      final disk = {'notes.xyz': _text('anything\nat all\n'), 'data': _text('[1, 2, {"a": 3}]')};
      await _open(tester, disk, 'notes.xyz');
      expect(_key('files text'), findsOneWidget);
      expect(find.text('anything'), findsOneWidget);
      await _open(tester, disk, 'data');
      expect(_key('tree rows'), findsOneWidget);
    });

    testWidgets('a switch changes the viewer and is remembered for the type, in a panel opened later', (tester) async {
      final backend = MemoryLayoutBackend();
      final registry = standardViewerRegistry(choices: ViewerChoices(backend: backend));
      final disk = {'a.json': _text(_json), 'b.json': _text('{"x": 1}'), 'c.yaml': _text(_yaml)};
      await _open(tester, disk, 'a.json', viewers: registry);
      expect(_key('tree rows'), findsOneWidget);
      await tester.tap(_key('viewer switch text'));
      await tester.pumpAndSettle();
      expect(_key('tree rows'), findsNothing);
      expect(_key('files text'), findsOneWidget);
      expect(registry.choices.chosen('application/json'), 'text');

      // Another JSON file, and a new panel on a registry that reads the same backend.
      final later = standardViewerRegistry(choices: ViewerChoices(backend: backend));
      await _open(tester, disk, 'b.json', viewers: later);
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget, reason: 'the choice came back from the backend');
      await tester.tap(_key('viewer switch tree'));
      await tester.pumpAndSettle();
      expect(_key('tree rows'), findsOneWidget);
      await tester.tap(_key('files back'));
      await tester.pumpAndSettle();
      await tester.tap(_key('files entry c.yaml'));
      await tester.pumpAndSettle();
      expect(_key('tree rows'), findsOneWidget, reason: 'YAML is another type, so it was not changed');
    });

    testWidgets('a collection opens and shuts, and a document that does not read as its format is said so and shown as text', (tester) async {
      final disk = {'a.json': _text(_json), 'broken.json': _text('{"a": 1,}')};
      await _open(tester, disk, 'a.json');
      expect(_key('tree value scripts/test'), findsOneWidget, reason: 'the first level under the root is open');
      await tester.tap(_key('tree row scripts'));
      await tester.pumpAndSettle();
      expect(_key('tree value scripts/test'), findsNothing);
      await tester.tap(_key('tree row scripts'));
      await tester.pumpAndSettle();
      expect(_key('tree value scripts/test'), findsOneWidget);

      await tester.tap(_key('files back'));
      await tester.pumpAndSettle();
      await tester.tap(_key('files entry broken.json'));
      await tester.pumpAndSettle();
      expect(_key('tree problem'), findsOneWidget);
      expect(find.textContaining('does not read as JSON'), findsOneWidget);
      expect(find.text('{"a": 1,}'), findsOneWidget);
    });

    testWidgets('a large file is cut as before: text says how much is shown, and a cut document is read as text', (tester) async {
      final big = 'line one\n${'x' * 90}\n';
      await _open(tester, {'a.txt': _text(big)}, 'a.txt', limit: 40);
      expect(_key('files truncated'), findsOneWidget);
      expect(_key('files text'), findsOneWidget);
      await _open(tester, {'a.json': _text(_json)}, 'a.json', limit: 40);
      expect(_key('files text'), findsOneWidget, reason: 'half a JSON document is no tree');
      expect(_key('tree rows'), findsNothing);
      expect(_key('files truncated'), findsOneWidget);
    });

    testWidgets('an image too large to read says so, and an image that claims too many pixels is not drawn', (tester) async {
      await _open(tester, {'a.png': _png}, 'a.png', limit: 20);
      expect(find.textContaining('too large to preview'), findsOneWidget);
      expect(_key('files too large'), findsOneWidget);
    });

    testWidgets('a viewer registered while the panel is up joins the switch', (tester) async {
      final registry = standardViewerRegistry()..types.addExtension('stamp', 'application/x-stamp');
      await _open(tester, {'a.stamp': _text('x')}, 'a.stamp', viewers: registry);
      expect(_key('viewer switch'), findsNothing);
      registry.register(const _Stamp());
      await tester.pumpAndSettle();
      expect(_key('stamp page'), findsOneWidget);
      expect(_key('viewer switch stamp'), findsOneWidget);
      expect(_key('viewer switch text'), findsOneWidget);
      registry.unregister('stamp');
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget);
    });
  });

  group('the diff reader', () {
    test('tells a file header, a hunk, an added and a removed line apart, and keeps what a long line is when it is cut', () {
      final rows = classifyDiff('--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same\n${'+${'y' * 900}'}\n');
      expect([for (final (row, _) in rows) row].take(7), [DiffRow.meta, DiffRow.meta, DiffRow.hunk, DiffRow.removed, DiffRow.added, DiffRow.context, DiffRow.added]);
      expect(rows.where((row) => row.$1 == DiffRow.added).length, 4, reason: 'a 901-character line is three pieces of an added line');
    });

    test('a line starting with -- inside a hunk is a removed line, not a header', () {
      final rows = classifyDiff('@@ -1 +1 @@\n--- a comment\n+++ more\n');
      expect([for (final (row, _) in rows) row].take(3), [DiffRow.hunk, DiffRow.removed, DiffRow.added]);
    });
  });

  group('the tree reader', () {
    test('reads JSON, YAML and TOML into the same maps and lists, and says what is wrong with a document that is none', () {
      expect(parseStructured(_json, StructuredFormat.json), containsPair('size', 12));
      expect(parseStructured(_yaml, StructuredFormat.yaml), {'name': 'canoa', 'ports': [80, 443], 'nested': {'deep': {'ok': true}}});
      expect(parseStructured(_toml, StructuredFormat.toml), {'title': 'canoa', 'host': {'port': 8080, 'names': ['a', 'b']}});
      expect(() => parseStructured('{"a": 1,}', StructuredFormat.json), throwsFormatException);
      expect(() => parseStructured('a: [1', StructuredFormat.yaml), throwsFormatException);
      expect(() => parseStructured('a = ', StructuredFormat.toml), throwsFormatException);
    });

    test('lists the rows that show: a shut collection hides its children', () {
      final root = {'a': {'b': 1}, 'c': [1, 2]};
      expect([for (final row in treeRows(root, {''})) row.id], ['', 'a', 'c']);
      expect([for (final row in treeRows(root, {'', 'a', 'c'})) row.id], ['', 'a', 'a/b', 'c', 'c/0', 'c/1']);
      expect(firstOpened(root), {'', 'a', 'c'});
    });
  });

  group('as drawn', () {
    final disk = {'package.json': _text(_json), 'changes.patch': _text(_diff), 'plan.md': _text('---\nstatus: open\n---\n# Plan\n\nWords.\n## Part\n\nMore.\n')};
    for (final (label, size) in [('desktop', const Size(560, 640)), ('phone', const Size(390, 780))]) {
      testWidgets('the tree, the diff and the sections at $label width', (tester) async {
        for (final (name, golden) in [('package.json', 'tree'), ('changes.patch', 'diff'), ('plan.md', 'markdown')]) {
          await _open(tester, disk, name, size: size);
          await expectLater(find.byType(FilesPanel), matchesGoldenFile('goldens/file_viewers_${golden}_$label.png'));
        }
      });
    }
  });

  group('highlighted', () {
    const rust = 'fn main() {\n    let x = 42; // answer\n    println!("hi");\n}\n';
    const patch = '--- a/lib/a.dart\n+++ b/lib/a.dart\n@@ -1,3 +1,3 @@\n void main() {\n-  var a = 1;\n+  var a = 2; // two\n }\n';

    /// The colour the first piece of text [text] is drawn in.
    Color? colorOf(WidgetTester tester, String text) {
      Color? found;
      // A selectable line keeps its spans in its editing controller.
      for (final field in tester.widgetList<EditableText>(find.byType(EditableText))) {
        final built = field.controller.buildTextSpan(context: tester.element(find.byWidget(field)), style: field.style, withComposing: false);
        built.visitChildren((span) {
          if (span is TextSpan && span.text == text && found == null) {
            found = span.style?.color;
            return false;
          }
          return true;
        });
      }
      return found;
    }

    Future<void> panel(WidgetTester tester, Map<String, Uint8List> files, Brightness brightness, Size size) async {
      await tester.binding.setSurfaceSize(size);
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: themeFor(LookId.canoa, brightness),
        home: Scaffold(body: FilesPanel(files: _Disk(files), root: _Disk.root)),
      ));
      await tester.pumpAndSettle();
    }

    for (final brightness in Brightness.values) {
      testWidgets('a source file in the ${brightness.name} look, by its extension', (tester) async {
        await panel(tester, {'main.rs': _text(rust), 'notes.xyz': _text(rust)}, brightness, const Size(560, 640));
        await tester.tap(_key('files entry main.rs'));
        await tester.pumpAndSettle();
        final look = Theme.of(tester.element(find.byType(Scaffold))).extension<ChatLook>()!;
        expect(colorOf(tester, 'fn'), look.accent);
        expect(colorOf(tester, '42'), look.warning);
        expect(colorOf(tester, '// answer'), look.muted);

        await tester.tap(_key('files back'));
        await tester.pumpAndSettle();
        await tester.tap(_key('files entry notes.xyz'));
        await tester.pumpAndSettle();
        expect(colorOf(tester, 'fn'), isNull, reason: 'a name and a start nothing recognises is shown plain');
        expect(_key('files line 0'), findsOneWidget);
      });

      testWidgets('a patch in the ${brightness.name} look colours the code of its file by that file\'s language', (tester) async {
        await panel(tester, {'c.patch': _text(patch)}, brightness, const Size(560, 640));
        await tester.tap(_key('files entry c.patch'));
        await tester.pumpAndSettle();
        final look = Theme.of(tester.element(find.byType(Scaffold))).extension<ChatLook>()!;
        expect(colorOf(tester, 'var'), look.accent);
        expect(colorOf(tester, '// two'), look.muted);
        expect(_key('diff added 5'), findsOneWidget, reason: 'the added line keeps its row colour');
      });
    }

    testWidgets('a large file stays responsive: only the lines in view are lexed', (tester) async {
      await tester.binding.setSurfaceSize(const Size(560, 640));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final big = [for (var i = 0; i < 40000; i++) 'let value_$i = $i; // line $i'].join('\n');
      final watch = Stopwatch()..start();
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(body: TextLines(text: big, language: languageForInfo('rust'))),
      ));
      await tester.pumpAndSettle();
      expect(watch.elapsedMilliseconds, lessThan(5000));
      expect(_key('files line 0'), findsOneWidget);
      expect(find.byType(SelectableText).evaluate().length, lessThan(80), reason: 'only the rows in view are built');
      expect(colorOf(tester, 'let'), isNotNull);

      // Past the default limit the same text is drawn plain, and as fast.
      final huge = [for (var i = 0; i < 100000; i++) 'let value_$i = $i; // line $i'].join('\n');
      watch.reset();
      await tester.pumpWidget(MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(body: TextLines(text: huge, language: languageForInfo('rust'))),
      ));
      await tester.pumpAndSettle();
      expect(watch.elapsedMilliseconds, lessThan(5000));
      expect(colorOf(tester, 'let'), isNull);
    });

    testWidgets('a text past the theme\'s limit is shown plain', (tester) async {
      await tester.binding.setSurfaceSize(const Size(560, 640));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final themed = themeFor(LookId.canoa, Brightness.dark);
      await tester.pumpWidget(MaterialApp(
        theme: themed.copyWith(extensions: [...themed.extensions.values, const SyntaxTheme(maxChars: 20)]),
        home: Scaffold(body: TextLines(text: rust, language: languageForInfo('rust'))),
      ));
      await tester.pumpAndSettle();
      expect(colorOf(tester, 'fn'), isNull);
    });

    for (final (label, size) in [('desktop', const Size(560, 640)), ('phone', const Size(390, 780))]) {
      for (final brightness in Brightness.values) {
        testWidgets('as drawn at $label width, ${brightness.name}', (tester) async {
          await panel(tester, {'main.rs': _text(rust), 'c.patch': _text(patch)}, brightness, size);
          for (final name in ['main.rs', 'c.patch']) {
            await tester.tap(_key('files entry $name'));
            await tester.pumpAndSettle();
            await expectLater(find.byType(FilesPanel), matchesGoldenFile('goldens/syntax_file_${name.split('.').last}_${label}_${brightness.name}.png'));
            await tester.tap(_key('files back'));
            await tester.pumpAndSettle();
          }
        });
      }
    }
  });
}
