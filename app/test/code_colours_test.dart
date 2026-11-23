// The package's code-line hook, set by the look: a changed file's lines in the Changes pane, a diff the permission card shows and git's
// changes view are coloured as the file's language says, in the light and the dark look, and are plain with no path or a language
// nothing is known of.
@TestOn('vm')
library;

import 'package:canoa/chat/git_changes.dart';
import 'package:canoa/chat/git_changes_pane.dart';
import 'package:canoa/highlight/highlighted.dart';
import 'package:canoa/theme/look.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

Widget _scene(Widget child, Brightness brightness, [LookId look = LookId.canoa]) => MaterialApp(
  debugShowCheckedModeBanner: false,
  theme: themeFor(look, brightness),
  home: Scaffold(body: SingleChildScrollView(child: child)),
);

/// The colour the first piece of text [text] is drawn in, or null when it is in no piece of its own.
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

ChatLook _lookOf(WidgetTester tester) => Theme.of(tester.element(find.byType(Scaffold))).extension<ChatLook>()!;

const _looks = [(LookId.canoa, Brightness.light), (LookId.canoa, Brightness.dark), (LookId.terminal, Brightness.light), (LookId.terminal, Brightness.dark)];

FileChange _calc(String path) => FileChange(path)
  ..whole = true
  ..before = 'function sum(a, b) {\n  return a - b;\n}'
  ..after = 'function sum(a, b) {\n  return a + b;\n}';

void main() {
  for (final (id, brightness) in _looks) {
    final name = '${id.name} ${brightness.name}';

    testWidgets('the Changes pane colours a known language in the $name look', (tester) async {
      await tester.pumpWidget(_scene(
        SizedBox(height: 400, child: ChangesPane(files: [_calc('src/calc.js')], isViewed: (_) => false, onViewed: (_, _) {}, onComment: (_) {})),
        brightness,
        id,
      ));
      final look = _lookOf(tester);
      expect(_colorOf(tester, 'function'), look.accent, reason: 'a keyword is in the look\'s accent');
      expect(_colorOf(tester, 'return'), look.accent);
    });

    testWidgets('the Changes pane leaves a file of no known language plain in the $name look', (tester) async {
      await tester.pumpWidget(_scene(
        SizedBox(height: 400, child: ChangesPane(files: [_calc('notes.unknownext')], isViewed: (_) => false, onViewed: (_, _) {}, onComment: (_) {})),
        brightness,
        id,
      ));
      expect(_colorOf(tester, 'function'), isNull);
      expect(find.textContaining('return a + b;', findRichText: true), findsOneWidget);
    });

    testWidgets('a diff with a path is coloured and one with none is plain in the $name look', (tester) async {
      final lines = withContext(diffLines('let a = 1;', 'let a = 2;'));
      await tester.pumpWidget(_scene(Column(children: [DiffView(lines: lines, path: 'src/a.rs')]), brightness, id));
      expect(_colorOf(tester, 'let'), _lookOf(tester).accent);
      await tester.pumpWidget(_scene(Column(children: [DiffView(lines: lines)]), brightness, id));
      expect(_colorOf(tester, 'let'), isNull);
    });

    testWidgets('git\'s changes view colours a known language in the $name look', (tester) async {
      const file = GitFile(
        path: 'src/lib.rs',
        status: 'M',
        added: 1,
        removed: 1,
        binary: false,
        patch: '@@ -1,2 +1,2 @@\n fn main() {\n-    let a = 1;\n+    let a = 2;\n }\n',
        cut: false,
        print: 'p',
      );
      await tester.pumpWidget(_scene(
        SizedBox(height: 500, child: GitChangesPane(scope: ChangeScope.uncommitted, files: const [file], more: false, onComment: (_) {})),
        brightness,
        id,
      ));
      expect(_colorOf(tester, 'let'), _lookOf(tester).accent);
      expect(_colorOf(tester, 'fn'), _lookOf(tester).accent);
    });
  }

  testWidgets('a line past the size cap is plain', (tester) async {
    final line = 'let a = "${'x' * eagerLimit}";';
    await tester.pumpWidget(_scene(Column(children: [DiffView(lines: [DiffLine(DiffKind.added, line)], path: 'a.rs')]), Brightness.dark));
    expect(_colorOf(tester, 'let'), isNull);
  });
}
