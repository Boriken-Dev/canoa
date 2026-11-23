// Syntax highlighting: the lexer, and the three places a conversation shows code (a message's code block, an edit's diff, a tool's
// output), in the light and the dark look, with the colours taken from the look and replaceable by a theme.
@TestOn('vm')
library;

import 'dart:io';

import 'package:canoa/highlight/highlighted.dart';
import 'package:canoa/highlight/syntax.dart';
import 'package:canoa/highlight/syntax_theme.dart';
import 'package:canoa/theme/look.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _rust = 'fn main() {\n    let x = 42; // answer\n    println!("hi {x}");\n}';

/// Every kind of text on [line] in [language], as `text:kind`.
List<String> _kinds(String language, String line, [LexState state = 0]) {
  final (spans, _) = lexLine(languageForInfo(language)!, line, state);
  return [for (final span in spans) '${line.substring(span.start, span.end)}:${span.tok.name}'];
}

Widget _scene(Widget child, Brightness brightness, {ThemeData? over}) => MaterialApp(
  debugShowCheckedModeBanner: false,
  theme: over ?? themeFor(LookId.canoa, brightness),
  home: Scaffold(body: Padding(padding: const EdgeInsets.all(16), child: SingleChildScrollView(child: child))),
);

/// The colour the first piece of text [text] is drawn in, or null when none is.
Color? _colorOf(WidgetTester tester, String text) {
  Color? found;
  for (final rich in tester.widgetList<RichText>(find.byType(RichText))) {
    rich.text.visitChildren((span) {
      if (span is TextSpan && span.text == text && found == null) {
        found = span.style?.color;
        return false;
      }
      return true;
    });
  }
  return found;
}

Future<ChatLook> _look(WidgetTester tester) async => Theme.of(tester.element(find.byType(Scaffold))).extension<ChatLook>()!;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/syntax_highlight_test.dart')));

  group('the lexer', () {
    test('sorts a line into keywords, literals, numbers, strings, comments and types', () {
      expect(_kinds('rust', 'let x: Option<u8> = None; // none'), [
        'let:keyword', 'Option:type', 'u8:type', 'None:literal', '// none:comment',
      ]);
      expect(_kinds('dart', "final a = 'it\\'s' + 3.5;"), ["final:keyword", "'it\\'s':string", '3.5:number']);
      expect(_kinds('python', '@app.route("/")'), ['@app:meta', '"/":string']);
      expect(_kinds('json', '{"a": [1, true, null], "b": "c"}'), ['"a":type', '1:number', 'true:literal', 'null:literal', '"b":type', '"c":string']);
      expect(_kinds('yaml', 'name: canoa # note'), ['name:type', '# note:comment']);
      expect(_kinds('toml', 'port = 8080'), ['port:type', '8080:number']);
      expect(_kinds('html', '<a href="x">'), ['a:keyword', '"x":string']);
      expect(_kinds('sql', 'SELECT id FROM t'), ['SELECT:keyword', 'FROM:keyword']);
    });

    test('carries a block comment and a multiline string to the next line', () {
      final c = languageForInfo('c')!;
      final (first, open) = lexLine(c, 'int a; /* one', 0);
      expect(first.last.tok, Tok.comment);
      final (second, shut) = lexLine(c, 'two */ int b;', open);
      expect(second.first.tok, Tok.comment);
      expect(shut, 0);
      expect(_kinds('c', 'two */ int b;', open), ['two */:comment', 'int:type']);

      final python = languageForInfo('python')!;
      final (_, inside) = lexLine(python, 'x = """start', 0);
      expect(inside, isNot(0));
      expect(lexLine(python, 'still inside', inside).$1.single.tok, Tok.string);
      expect(lexLine(python, 'end""" + y', inside).$2, 0);
    });

    test('does not read a lifetime as a string, or a digit inside a name as a number', () {
      expect(_kinds('rust', "fn f<'a>(x: &'a str)"), ['fn:keyword', 'str:type']);
      expect(_kinds('dart', 'var x2 = 7;'), ['var:keyword', '7:number']);
    });

    test('names the language from a fence, a path or what the text starts with, and is plain otherwise', () {
      expect(languageForInfo('rust')?.id, 'rust');
      expect(languageForInfo('{.py}')?.id, 'python');
      expect(languageForInfo('language-ts title="x"')?.id, 'typescript');
      expect(languageForInfo('JSON')?.id, 'json');
      expect(languageForInfo('klingon'), isNull);
      expect(languageForInfo(''), isNull);
      expect(languageForPath('/a/b/main.rs')?.id, 'rust');
      expect(languageForPath('C:\\a\\Dockerfile')?.id, 'shell');
      expect(languageForPath('notes.xyz'), isNull);
      expect(languageForPath('.gitignore')?.id, 'shell');
      expect(sniffLanguage('#!/usr/bin/env python3\nprint(1)')?.id, 'python');
      expect(sniffLanguage('#!/bin/bash\nls')?.id, 'shell');
      expect(sniffLanguage('{"a": 1}')?.id, 'json');
      expect(sniffLanguage('<?xml version="1.0"?><a/>')?.id, 'xml');
      expect(sniffLanguage('--- a/x\n+++ b/x\n@@ -1 +1 @@\n')?.id, 'diff');
      expect(sniffLanguage('just some words'), isNull);
    });

    test('lexes only the lines asked for, from a checkpoint, and not at all past the limit', () {
      final lines = [for (var i = 0; i < 200000; i++) i % 50 == 0 ? '/* block $i' : 'let x$i = $i; */'];
      final lexed = HighlightedLines(lines, languageForInfo('rust'));
      lexed.spansAt(10);
      expect(lexed.lexed, lessThan(100), reason: 'a line near the top costs its own block');
      lexed.spansAt(150000);
      expect(lexed.lexed, lessThan(200000 + 100), reason: 'reaching a far line lexes up to it once');
      final before = lexed.lexed;
      lexed.spansAt(150001);
      expect(lexed.lexed - before, lessThan(70), reason: 'the next line costs one checkpoint block at most');

      final capped = HighlightedLines(['let x = 1;'], languageForInfo('rust'), maxChars: 5);
      expect(capped.highlighted, isFalse);
      expect(capped.spansAt(0), isEmpty);
      expect(HighlightedLines(['x'], null).highlighted, isFalse);
    });
  });

  group('a code block', () {
    const message = 'Here:\n\n```rust\n$_rust\n```\n\nand\n\n```\n{"a": 1}\n```\n\n```text\nfn nothing\n```\n';

    for (final brightness in Brightness.values) {
      testWidgets('is coloured from the ${brightness.name} look', (tester) async {
        await tester.pumpWidget(_scene(AgentMarkdown(text: message, builders: highlightedMarkdown), brightness));
        final look = await _look(tester);
        expect(_colorOf(tester, 'fn'), look.accent, reason: 'a keyword is the accent');
        expect(_colorOf(tester, '42'), look.warning, reason: 'a number is the warning colour');
        expect(_colorOf(tester, '"hi {x}"'), look.success, reason: 'a string is the success colour');
        expect(_colorOf(tester, '// answer'), look.muted, reason: 'a comment is the quiet colour');
        expect(_colorOf(tester, '1'), look.warning, reason: 'a block with no language is read by what it looks like (JSON)');
        expect(find.text('fn nothing', findRichText: true), findsOneWidget, reason: 'a language nothing knows is plain, in one piece');
      });
    }

    testWidgets('takes different colours in the light and the dark look', (tester) async {
      await tester.pumpWidget(_scene(AgentMarkdown(text: '```rust\n$_rust\n```', builders: highlightedMarkdown), Brightness.dark));
      final dark = _colorOf(tester, 'fn');
      await tester.pumpWidget(_scene(AgentMarkdown(text: '```rust\n$_rust\n```', builders: highlightedMarkdown), Brightness.light));
      await tester.pumpAndSettle();
      expect(_colorOf(tester, 'fn'), isNot(dark));
    });

    testWidgets('takes a theme\'s own colours, and is plain past the theme\'s limit', (tester) async {
      const mine = Color(0xFF123456);
      final themed = themeFor(LookId.canoa, Brightness.dark);
      final replaced = themed.copyWith(extensions: [...themed.extensions.values, const SyntaxTheme(colors: {Tok.keyword: mine})]);
      await tester.pumpWidget(_scene(AgentMarkdown(text: '```rust\n$_rust\n```', builders: highlightedMarkdown), Brightness.dark, over: replaced));
      expect(_colorOf(tester, 'fn'), mine);

      final small = themed.copyWith(extensions: [...themed.extensions.values, const SyntaxTheme(maxChars: 10)]);
      await tester.pumpWidget(_scene(AgentMarkdown(text: '```rust\n$_rust\n```', builders: highlightedMarkdown), Brightness.dark, over: small));
      await tester.pump();
      expect(find.text(_rust, findRichText: true), findsOneWidget);
    });

    testWidgets('copies the code, not its colours', (tester) async {
      await tester.pumpWidget(_scene(AgentMarkdown(text: '```rust\n$_rust\n```', builders: highlightedMarkdown), Brightness.dark));
      expect(tester.widget<CopyButton>(find.byType(CopyButton)).text, _rust);
    });

    for (final (label, size) in [('desktop', const Size(900, 700)), ('phone', const Size(390, 780))]) {
      for (final brightness in Brightness.values) {
        testWidgets('as drawn at $label width, ${brightness.name}', (tester) async {
          await tester.binding.setSurfaceSize(size);
          addTearDown(() => tester.binding.setSurfaceSize(null));
          await tester.pumpWidget(_scene(AgentMarkdown(text: message, builders: highlightedMarkdown), brightness));
          await expectLater(find.byType(Scaffold), matchesGoldenFile('goldens/syntax_code_${label}_${brightness.name}.png'));
        });
      }
    }
  });

  group('an edit and a tool\'s output', () {
    final tool = ToolCall('t1')
      ..apply({
        'toolCallId': 't1',
        'title': 'Edit src/calc.rs',
        'kind': 'edit',
        'status': 'completed',
        'locations': [{'path': 'src/calc.rs', 'line': 1}],
        'content': [
          {'type': 'diff', 'path': 'src/calc.rs', 'oldText': 'fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n', 'newText': 'fn add(a: i32, b: i32) -> i32 {\n    a + b // sum\n}\n'},
          {'type': 'content', 'content': {'type': 'text', 'text': 'pub fn add() -> u8 {\n    "text"\n}'}},
        ],
      }, const ChatLimits());

    Widget blocks(ToolCall call) => ChatScope(
      renderers: highlightRenderers,
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [for (final block in call.blocks) ToolBlockView(tool: call, block: block)]),
    );

    Widget transcript(Brightness b) => _scene(blocks(tool), b);

    for (final brightness in Brightness.values) {
      testWidgets('colour the diff and the output from the ${brightness.name} look', (tester) async {
        await tester.pumpWidget(transcript(brightness));
        await tester.pumpAndSettle();
        final look = await _look(tester);
        expect(find.byType(HighlightedDiffView), findsOneWidget);
        expect(find.byType(CodeOutput), findsOneWidget);
        expect(_colorOf(tester, 'fn'), look.accent);
        expect(_colorOf(tester, '// sum'), look.muted);
        expect(_colorOf(tester, '"text"'), look.success);
      });
    }

    testWidgets('are left to the package when the file has no known language and the text looks like nothing', (tester) async {
      final plain = ToolCall('t2')
        ..apply({
          'toolCallId': 't2',
          'title': 'Read notes.xyz',
          'kind': 'read',
          'status': 'completed',
          'locations': [{'path': 'notes.xyz'}],
          'content': [
            {'type': 'diff', 'path': 'notes.xyz', 'oldText': 'a\n', 'newText': 'b\n'},
            {'type': 'content', 'content': {'type': 'text', 'text': 'some words'}},
          ],
        }, const ChatLimits());
      await tester.pumpWidget(_scene(blocks(plain), Brightness.dark));
      await tester.pumpAndSettle();
      expect(find.byType(HighlightedDiffView), findsNothing);
      expect(find.byType(CodeOutput), findsNothing);
      expect(find.byType(DiffView), findsOneWidget);
      expect(find.byType(CodeBox), findsOneWidget);
    });

    test('a patch is read by the file its header names', () {
      expect(languageOfPatch('--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a\n+b\n')?.id, 'python');
      expect(languageOfPatch('--- a/x\n+++ b/x\n@@ -1 +1 @@\n'), isNull);
    });

    for (final (label, size) in [('desktop', const Size(900, 700)), ('phone', const Size(390, 780))]) {
      for (final brightness in Brightness.values) {
        testWidgets('as drawn at $label width, ${brightness.name}', (tester) async {
          await tester.binding.setSurfaceSize(size);
          addTearDown(() => tester.binding.setSurfaceSize(null));
          await tester.pumpWidget(transcript(brightness));
          await tester.pumpAndSettle();
          await expectLater(find.byType(Scaffold), matchesGoldenFile('goldens/syntax_tool_${label}_${brightness.name}.png'));
        });
      }
    }
  });
}
