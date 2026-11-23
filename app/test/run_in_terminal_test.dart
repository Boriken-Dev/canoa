// "Run in terminal" under an agent's command card: it types the command into the person's terminal and nothing else, and is not there
// when no terminal is.
@TestOn('vm')
library;

import 'package:canoa/chat/run_in_terminal.dart';
import 'package:canoa/theme/look.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatLimits, ToolCall;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

ToolCall _call(String kind, String title) => ToolCall('t-$kind')..apply({'title': title, 'kind': kind, 'status': 'completed'}, const ChatLimits());

Future<void> _show(WidgetTester tester, ChatRenderers renderers, ToolCall call) => tester.pumpWidget(
  MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: ChatScope(renderers: renderers, child: ToolEntry(tool: call)),
    ),
  ),
);

void main() {
  testWidgets('a command card offers Run in terminal, which types the command and presses nothing', (tester) async {
    final typed = <String>[];
    await _show(tester, terminalRenderers(offered: () => true, run: typed.add), _call('execute', 'cargo test -p canoa-host'));
    expect(find.text('cargo test -p canoa-host'), findsOneWidget, reason: 'the package\'s own card is still drawn');
    await tester.tap(find.byKey(const ValueKey('run in terminal t-execute')));
    expect(typed, ['cargo test -p canoa-host']);
  });

  testWidgets('no terminal, no action; a card of another kind has none', (tester) async {
    await _show(tester, terminalRenderers(offered: () => false, run: (_) {}), _call('execute', 'ls'));
    expect(find.byKey(const ValueKey('run in terminal t-execute')), findsNothing);
    expect(find.text('ls'), findsOneWidget);
    await _show(tester, terminalRenderers(offered: () => true, run: (_) {}), _call('edit', 'Edit a.txt'));
    expect(find.text('Run in terminal'), findsNothing);
  });
}
