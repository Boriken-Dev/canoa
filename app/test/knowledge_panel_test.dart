// The Hosts section's knowledge search settings, `[host.kb]`: an address needs its model and a model its address, a saved
// pair reaches canoa.cfg beside the keys the app does not edit, and a refusal from the host is shown.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/knowledge_panel.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final saved = <CanoaConfig>[];
  String? refuseWith;

  Future<void> show(WidgetTester tester, CanoaConfig config) async {
    saved.clear();
    refuseWith = null;
    // The page is long: every part of it is on screen.
    await tester.binding.setSurfaceSize(const Size(900, 3000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
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
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String key, String text) async {
    await tester.enterText(find.byKey(Key('host-kb-$key')), text);
    await tester.pumpAndSettle();
  }

  bool canSave(WidgetTester tester) =>
      tester
          .widget<FilledButton>(find.byKey(const Key('host-kb-save')))
          .onPressed !=
      null;
  String note(WidgetTester tester) =>
      tester.widget<Text>(find.byKey(const Key('host-kb-note'))).data!;

  testWidgets(
    'what the file holds is shown, and nothing can be saved until something changes',
    (tester) async {
      await show(
        tester,
        CanoaConfig.parse(
          '[host.kb.embedder]\nuse = "openai:work"\nmodel = "embed-1"\n',
        ),
      );
      String held(String key) => tester
          .widget<TextField>(find.byKey(Key('host-kb-$key')))
          .controller!
          .text;
      expect(held('embedder_use'), 'openai:work');
      expect(held('embedder_model'), 'embed-1');
      expect(held('ranker_model'), isEmpty);
      expect(held('server'), isEmpty);
      expect(canSave(tester), isFalse);
    },
  );

  testWidgets(
    'a server that is no http or https address cannot be saved and says why',
    (tester) async {
      await show(tester, const CanoaConfig());
      await type(tester, 'server', 'ftp://host');
      expect(canSave(tester), isFalse);
      expect(note(tester), contains('http or https'));
      await type(tester, 'server', 'http://127.0.0.1:60570');
      expect(canSave(tester), isTrue);
      await type(tester, 'server', '');
      expect(canSave(tester), isFalse, reason: 'back to what the file holds');
    },
  );

  testWidgets(
    'a saved choice reaches canoa.cfg beside the keys the app does not edit',
    (tester) async {
      await show(tester, CanoaConfig.parse('[host.kb]\nchunk_bytes = 512\n'));
      await type(tester, 'server', ' http://127.0.0.1:60570 ');
      await type(tester, 'embedder_model', 'embed-1');
      await type(tester, 'ranker_use', 'openai:work');
      await type(tester, 'ranker_model', 'qwen');
      await tester.tap(find.byKey(const Key('host-kb-save')));
      await tester.pumpAndSettle();
      final reread = CanoaConfig.parse(saved.single.encode());
      expect(reread.hostKb, {
        'embedder_use': 'knowledge',
        'embedder_model': 'embed-1',
        'ranker_use': 'openai:work',
        'ranker_model': 'qwen',
        'server': 'http://127.0.0.1:60570',
      });
      expect((reread.host!['kb'] as Map)['chunk_bytes'], 512);
      expect(note(tester), contains('Saved'));
      expect(
        canSave(tester),
        isFalse,
        reason: 'what is shown is what is saved',
      );

      // Clearing the ranker's model turns it off and leaves the rest.
      await type(tester, 'ranker_model', '');
      await tester.tap(find.byKey(const Key('host-kb-save')));
      await tester.pumpAndSettle();
      expect(CanoaConfig.parse(saved.last.encode()).hostKb.keys, [
        'embedder_use',
        'embedder_model',
        'server',
      ]);
    },
  );

  testWidgets(
    'a refusal from the host is shown and the settings stay unsaved',
    (tester) async {
      await show(tester, const CanoaConfig());
      refuseWith = 'canoa.cfg is not right for it';
      await type(tester, 'embedder_model', 'embed-1');
      await tester.tap(find.byKey(const Key('host-kb-save')));
      await tester.pumpAndSettle();
      expect(saved, isEmpty);
      expect(note(tester), 'canoa.cfg is not right for it');
      expect(
        canSave(tester),
        isTrue,
        reason: 'the person may correct it and try again',
      );
    },
  );
}
