// A Markdown file as sections: how it is parsed (fences, headings, frontmatter, anchors), and how the Files panel chooses between the
// sectioned view and the plain one, and opens a file whose folder name holds a `#`.
@TestOn('vm')
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/surfaces/markdown_section_viewer.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Disk implements WorkspaceFiles {
  _Disk(this.home);

  final Directory home;

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    final dir = Directory(path ?? home.path);
    if (!dir.existsSync()) throw const ControlFailed(-32003, 'no such folder');
    final entries = [
      for (final each in dir.listSync()..sort((a, b) => a.path.compareTo(b.path)))
        if (each is Directory || files)
          HostFolderEntry(name: each.uri.pathSegments.where((part) => part.isNotEmpty).last, path: each.path, isFolder: each is Directory),
    ];
    final parent = dir.parent.path;
    return HostFolderListing(path: dir.path, parent: parent == dir.path ? null : parent, entries: entries, more: false);
  }

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    final bytes = File(path).readAsBytesSync();
    return HostFileRead(path: path, name: path.split(RegExp(r'[\\/]')).last, size: bytes.length, kind: 'text', truncated: false, bytes: Uint8List.fromList(bytes));
  }
}

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _show(WidgetTester tester, Widget child) async {
  await tester.binding.setSurfaceSize(const Size(700, 760));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: child)));
  await tester.pumpAndSettle();
}

const _doc = '''---
name: sample
status: ready
tags: [markdown, viewer]
owner:
  name: alice
  team: core
list:
  - one
  - two
---

# Title

Intro.

## Progress

- [ ] First

### Part A

Body A.

### Part B

Body B.
''';

File _write(Directory dir, String name, String text) {
  final file = File('${dir.path}/$name');
  file.parent.createSync(recursive: true);
  return file..writeAsStringSync(text);
}

void main() {
  group('parsing', () {
    test('frontmatter keeps its top-level keys and shows a nested block or a list as the file wrote it', () {
      final parsed = parseMarkdownSections(_doc);
      expect({for (final e in parsed.frontmatter) e.key: e.value}, {
        'name': 'sample',
        'status': 'ready',
        'tags': '[markdown, viewer]',
        'owner': 'name: alice\n  team: core',
        'list': '- one\n  - two',
      });
      expect(parsed.sections.map((s) => s.slug), ['title', 'progress', 'part-a', 'part-b']);
    });

    test('a heading in a fence is text, and a fence closes only with its own mark at least as long', () {
      final parsed = parseMarkdownSections('# A\n````\n```\n# not a heading\n```\n# still inside\n````\n# B\n~~~\n# in tildes\n```\n# still in tildes\n~~~\n# C\n');
      expect(parsed.sections.map((s) => s.title), ['A', 'B', 'C']);
      expect(parsed.sections.first.body, contains('# still inside'));
      expect(parsed.sections[1].body, contains('# still in tildes'));
    });

    test('headings run from one to six marks and are not indented code', () {
      final parsed = parseMarkdownSections('# One\n###### Six\n####### seven\n    # indented code\n##### Five ##\n');
      expect(parsed.sections.map((s) => (s.level, s.title)), [(1, 'One'), (6, 'Six'), (5, 'Five')]);
    });

    test('an anchor is the exact slug or the exact title, never a prefix', () {
      final parsed = parseMarkdownSections('# Plan\n## Part 2\n## Part 20\n## Part 2\n');
      expect(parsed.findSection('part-2')!.title, 'Part 2');
      expect(parsed.findSection('#part-20')!.slug, 'part-20');
      expect(parsed.findSection('Part 2')!.slug, 'part-2');
      expect(parsed.findSection('part-2-2')!.slug, 'part-2-2');
      expect(parsed.findSection('part'), isNull);
      expect(parsed.findSection('par'), isNull);
    });

    test('a file with frontmatter or three headings is structured, a short one is not', () {
      expect(parseMarkdownSections(_doc).isStructured, isTrue);
      expect(parseMarkdownSections('# A\n## B\n### C\n').isStructured, isTrue);
      expect(parseMarkdownSections('# A\n## B\ntext\n').isStructured, isFalse);
      expect(parseMarkdownSections('plain\n').isStructured, isFalse);
    });
  });

  group('the Files panel', () {
    testWidgets('shows the sectioned view of a structured file, with its frontmatter and an outline', (tester) async {
      final home = Directory.systemTemp.createTempSync('canoa-md-');
      addTearDown(() => home.deleteSync(recursive: true));
      final file = _write(home, 'plan.md', _doc);
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(file.path)));
      expect(_key('markdown section viewer'), findsOneWidget);
      expect(_key('markdown frontmatter status'), findsOneWidget);
      expect(find.text('[markdown, viewer]'), findsOneWidget);
      expect(_key('markdown outline part-b'), findsOneWidget);
      await tester.tap(_key('markdown outline part-b'));
      await tester.pumpAndSettle();
      expect(_key('markdown focused part-b'), findsOneWidget);
      await tester.tap(_key('markdown heading part-b'));
      await tester.pumpAndSettle();
      expect(_key('markdown body part-b'), findsNothing);
    });

    testWidgets('a plain Markdown file keeps its plain view until the person asks for sections, and can go back', (tester) async {
      final home = Directory.systemTemp.createTempSync('canoa-md-');
      addTearDown(() => home.deleteSync(recursive: true));
      final file = _write(home, 'notes.md', '# Notes\n\nA few words.\n');
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(file.path)));
      expect(_key('files text'), findsOneWidget);
      expect(_key('markdown section viewer'), findsNothing);
      await tester.tap(_key('viewer switch markdown'));
      await tester.pumpAndSettle();
      expect(_key('markdown section viewer'), findsOneWidget);
      await tester.tap(_key('viewer switch text'));
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget);
    });

    testWidgets('a structured file can be read plain', (tester) async {
      final home = Directory.systemTemp.createTempSync('canoa-md-');
      addTearDown(() => home.deleteSync(recursive: true));
      final file = _write(home, 'plan.md', _doc);
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(file.path)));
      await tester.tap(_key('viewer switch text'));
      await tester.pumpAndSettle();
      expect(_key('files text'), findsOneWidget);
      expect(_key('markdown frontmatter'), findsNothing);
    });

    testWidgets('a reveal names its section apart from the path, so a # in a folder or file name opens', (tester) async {
      final home = Directory.systemTemp.createTempSync('canoa-md-');
      addTearDown(() => home.deleteSync(recursive: true));
      final inFolder = _write(home, 'c#/notes.md', '# Notes\n\nA few words.\n');
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(inFolder.path)));
      expect(find.textContaining('A few words.'), findsOneWidget);
      final named = _write(home, 'a#b.md', _doc);
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(named.path, section: 'part-b')));
      expect(find.text('a#b.md'), findsOneWidget);
      expect(_key('markdown focused part-b'), findsOneWidget);
    });

    testWidgets('a section asked for opens a short file sectioned too', (tester) async {
      final home = Directory.systemTemp.createTempSync('canoa-md-');
      addTearDown(() => home.deleteSync(recursive: true));
      final file = _write(home, 'short.md', '# Top\n\ntext\n## Second\n\nmore\n');
      await _show(tester, FilesPanel(files: _Disk(home), root: home.path, reveal: FileReveal(file.path, section: 'second')));
      expect(_key('markdown focused second'), findsOneWidget);
    });
  });
}
