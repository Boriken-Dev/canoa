// The Hosts section's knowledge search settings, `[host.kb]`: an address needs its model and a model its address, a saved
// pair reaches canoa.cfg beside the keys the app does not edit, and a refusal from the host is shown.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/knowledge_panel.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final saved = <CanoaConfig>[];
  String? refuseWith;

  Future<void> show(WidgetTester tester, CanoaConfig config) async {
    saved.clear();
    refuseWith = null;
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: KnowledgeSearchPanel(
            config: config,
            onSave: (changed) async {
              if (refuseWith != null) return refuseWith;
              saved.add(changed);
              return null;
            },
          ),
        ),
      ),
    ));
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String key, String text) async {
    await tester.enterText(find.byKey(Key('host-kb-$key')), text);
    await tester.pumpAndSettle();
  }

  bool canSave(WidgetTester tester) => tester.widget<FilledButton>(find.byKey(const Key('host-kb-save'))).onPressed != null;
  String note(WidgetTester tester) => tester.widget<Text>(find.byKey(const Key('host-kb-note'))).data!;

  testWidgets('what the file holds is shown, and nothing can be saved until something changes', (tester) async {
    await show(tester, CanoaConfig.parse('[host.kb]\nembeddings = "http://127.0.0.1:60570"\nmodel = "embed-1"\n'));
    String held(String key) => tester.widget<TextField>(find.byKey(Key('host-kb-$key'))).controller!.text;
    expect(held('embeddings'), 'http://127.0.0.1:60570');
    expect(held('model'), 'embed-1');
    expect(held('llm_endpoint'), isEmpty);
    expect(canSave(tester), isFalse);
  });

  testWidgets('an address with no model, or a model with no address, or a wrong scheme, cannot be saved and says why', (tester) async {
    await show(tester, const CanoaConfig());
    await type(tester, 'llm_endpoint', 'http://127.0.0.1:60570');
    expect(canSave(tester), isFalse);
    expect(note(tester), contains('chat model needs the exact model'));
    await type(tester, 'llm_model', 'qwen');
    expect(canSave(tester), isTrue);
    await type(tester, 'llm_endpoint', 'ftp://host');
    expect(canSave(tester), isFalse);
    expect(note(tester), contains('http or https'));
    await type(tester, 'llm_endpoint', '');
    expect(note(tester), contains('needs the address'));
    await type(tester, 'llm_model', '');
    expect(canSave(tester), isFalse, reason: 'back to what the file holds');
    await type(tester, 'model', 'embed-1');
    expect(note(tester), contains('embeddings server needs the address'));
  });

  testWidgets('a saved pair reaches canoa.cfg beside the keys of the table the app does not edit', (tester) async {
    await show(tester, CanoaConfig.parse('[host.kb]\nchunk_bytes = 512\n'));
    await type(tester, 'embeddings', ' http://127.0.0.1:60570 ');
    await type(tester, 'model', 'embed-1');
    await type(tester, 'llm_endpoint', 'http://127.0.0.1:60570');
    await type(tester, 'llm_model', 'qwen');
    await tester.tap(find.byKey(const Key('host-kb-save')));
    await tester.pumpAndSettle();
    final reread = CanoaConfig.parse(saved.single.encode());
    expect(reread.hostKb, {
      'embeddings': 'http://127.0.0.1:60570',
      'model': 'embed-1',
      'llm_endpoint': 'http://127.0.0.1:60570',
      'llm_model': 'qwen',
    });
    expect((reread.host!['kb'] as Map)['chunk_bytes'], 512);
    expect(note(tester), contains('Saved'));
    expect(canSave(tester), isFalse, reason: 'what is shown is what is saved');

    // Clearing the pair takes the keys out and leaves the rest.
    await type(tester, 'llm_endpoint', '');
    await type(tester, 'llm_model', '');
    await tester.tap(find.byKey(const Key('host-kb-save')));
    await tester.pumpAndSettle();
    expect(CanoaConfig.parse(saved.last.encode()).hostKb.keys, ['embeddings', 'model']);
  });

  testWidgets('a refusal from the host is shown and the settings stay unsaved', (tester) async {
    await show(tester, const CanoaConfig());
    refuseWith = 'canoa.cfg is not right for it';
    await type(tester, 'embeddings', 'http://127.0.0.1:60570');
    await type(tester, 'model', 'embed-1');
    await tester.tap(find.byKey(const Key('host-kb-save')));
    await tester.pumpAndSettle();
    expect(saved, isEmpty);
    expect(note(tester), 'canoa.cfg is not right for it');
    expect(canSave(tester), isTrue, reason: 'the person may correct it and try again');
  });
}
