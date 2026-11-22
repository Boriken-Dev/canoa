import 'dart:io';

import 'package:canoa/chat/conversation_export.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_session_agent.dart';

void main() {
  testWidgets('copy as Markdown and export as Markdown from the header', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    String? copied;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') {
        copied = (call.arguments as Map)['text'] as String?;
      }
      return null;
    });
    addTearDown(() => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));

    final dir = Directory.systemTemp.createTempSync('canoa_ux_export_');
    addTearDown(() => dir.deleteSync(recursive: true));

    String? exportedName;
    String? exportedMarkdown;
    final store = SessionStore();
    final agent = FakeAgent();
    await tester.runAsync(() => agent.start(store, cwd: dir.path));

    markdownExporterForTest = (name, md) async {
      exportedName = name;
      exportedMarkdown = md;
      return true;
    };
    addTearDown(() => markdownExporterForTest = null);
    await tester.pumpWidget(CanoaApp(
      store: store,
      config: const CanoaConfig(density: ChatDensity.verbose),
      surfaces: [FilesSurface((_) => FakeFiles())],
    ));
    agent.update({'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': 'Check the alpha token.'}});
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Found `alpha` inside **config**.'}});
    await settle(tester);
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Copy conversation as Markdown'));
    await tester.pumpAndSettle();
    expect(copied, contains('# Fix addNumbers bug'));
    expect(copied, contains('## Claude Code'));
    expect(copied, contains('Found `alpha` inside **config**.'));

    await tester.tap(find.byTooltip('Export conversation as Markdown…'));
    await tester.pumpAndSettle();
    expect(exportedName, 'Fix-addNumbers-bug.md');
    expect(exportedMarkdown, contains('Check the alpha token.'));
    store.dispose();
  });
}
