import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/chat/file_links.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('resolveFileLink and looksLikeFilePath distinguish paths from web links and prose', () {
    expect(resolveFileLink('src/calc.js', root: '/work/proj'), '/work/proj/src/calc.js');
    expect(resolveFileLink('src/calc.js:42:7', root: '/work/proj'), '/work/proj/src/calc.js');
    expect(resolveFileLink('lib/main.dart#L10-L20', root: 'C:\\work\\proj'), 'C:\\work\\proj\\lib\\main.dart');
    expect(resolveFileLink('https://example.org/src/calc.js', root: '/work/proj'), isNull);
    expect(resolveFileLink('mailto:ana@example.test', root: '/work/proj'), isNull);
    expect(resolveFileLink('#section', root: '/work/proj'), isNull);
    expect(looksLikeFilePath('app/lib/main.dart:12'), isTrue);
    expect(looksLikeFilePath('hello.txt'), isTrue);
    expect(looksLikeFilePath('addNumbers(a, b)'), isFalse);
    expect(looksLikeFilePath('https://example.org'), isFalse);
  });

  testWidgets('filePathRenderers opens files from Markdown links, inline code, tool cards, and diff headers', (tester) async {
    final opened = <String>[];
    String? copied;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') {
        copied = (call.arguments as Map)['text'] as String?;
      }
      return null;
    });
    addTearDown(() => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));

    final tool = ToolCall('t1')
      ..apply({
        'toolCallId': 't1',
        'title': 'Edit src/calc.js',
        'kind': 'edit',
        'status': 'failed',
        'locations': [{'path': 'src/calc.js', 'line': 12}],
        'content': [{'type': 'diff', 'path': 'src/calc.js', 'oldText': 'a - b\n', 'newText': 'a + b\n'}],
      }, const ChatLimits());

    final entries = [
      ChatEntry(
        MessageRole.agent,
        'See [calc link](src/calc.js:12), inline `lib/helper.dart`, and [docs](https://example.org/docs).',
        at: DateTime(2026, 11, 22, 10, 0),
      ),
      ChatEntry(
        MessageRole.tool,
        '',
        at: DateTime(2026, 11, 22, 10, 1),
        tool: tool,
      ),
    ];

    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: ChatTranscript(
          entries: entries,
          renderers: filePathRenderers(onOpenFile: opened.add, root: '/repo'),
        ),
      ),
    ));

    void tapSpanText(String target) {
      TapGestureRecognizer? recognizer;
      for (final text in tester.widgetList<RichText>(find.byType(RichText))) {
        text.text.visitChildren((span) {
          if (span is TextSpan && span.toPlainText() == target && span.recognizer is TapGestureRecognizer) {
            recognizer = span.recognizer! as TapGestureRecognizer;
            return false;
          }
          return true;
        });
        if (recognizer != null) break;
      }
      expect(recognizer, isNotNull, reason: 'missing link recognizer for $target');
      recognizer!.onTap!();
    }

    tapSpanText('calc link');
    await tester.pump();
    expect(opened, ['/repo/src/calc.js']);

    tapSpanText('lib/helper.dart');
    await tester.pump();
    expect(opened, ['/repo/src/calc.js', '/repo/lib/helper.dart']);

    tapSpanText('docs');
    await tester.pump();
    expect(copied, 'https://example.org/docs');
    expect(opened.length, 2);

    await tester.tap(find.byKey(const ValueKey('tool-open-file /repo/src/calc.js')));
    await tester.pump();
    expect(opened.last, '/repo/src/calc.js');

    await tester.tap(find.byKey(const ValueKey('diff-file-link src/calc.js')));
    await tester.pump();
    expect(opened.length, 4);
  });
}
