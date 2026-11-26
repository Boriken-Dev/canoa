// Settings > Knowledge search, what the host offers: for each operation who can do it and in what order the host asks, the backend
// and model of the matcher and the ranker picked from the offers (the models split by what they report), a choice tried with the
// host, the agents on their own login and the warm sessions they keep, saved to canoa.cfg, in the person's language.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/kb_offers.dart';
import 'package:canoa/desktop/knowledge_panel.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

KbOffers _offers({String? model}) => KbOffers.parse({
  'operations': [
    for (final (id, capability) in [('embed', 'embed'), ('rerank', 'rerank'), ('complete', 'chat')])
      {
        'id': id,
        'title': id,
        'capability': capability,
        'use': null,
        'fallback': <String>[],
        'asked': id == 'rerank' ? <String>[] : ['openai:work'],
        'problem': id == 'rerank' ? 'no backend is set up to rerank' : null,
        'offers': [
          if (id != 'rerank')
            {
              'id': 'openai:work',
              'title': 'OpenAI, account work',
              'kind': 'account',
              'location': 'Text goes to OpenAI.',
              'address': 'https://api.openai.com/v1',
              if (model == 'openai:work')
                'models': {
                  'listed': true,
                  'offered': [
                    {'id': id == 'embed' ? 'text-embedding-3-small' : 'gpt-5-mini', 'capabilities': [capability]},
                  ],
                  'unreported': [
                    {'id': 'mystery-1', 'capabilities': <String>[]},
                  ],
                },
            },
          if (id == 'complete') {'id': 'harness:claude', 'title': 'Claude Code', 'kind': 'harness', 'location': 'Run here under its own login.'},
        ],
      },
  ],
  'harnesses': [
    {
      'source': 'Claude',
      'accounts': ['personal'],
      'profiles': ['terse'],
    },
  ],
  'pool': {'idle_seconds': 120, 'max_sessions': 2},
})!;

void main() {
  final saved = <CanoaConfig>[];
  final tried = <Map<String, Object?>>[];

  KbHost host() => KbHost(
    offers: ([offer]) async => _offers(model: offer),
    test: (params) async {
      tried.add(params);
      return params['role'] == 'embedder' ? {'ok': true, 'length': 1536, 'ms': 120} : {'ok': false, 'why': 'the account needs a new sign-in'};
    },
  );

  Future<void> show(WidgetTester tester, CanoaConfig config, {String? language}) async {
    saved.clear();
    tried.clear();
    await tester.binding.setSurfaceSize(const Size(900, 3000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => AppTextHost(language: language, child: child!),
        home: Scaffold(
          body: SingleChildScrollView(
            child: KnowledgeSearchPanel(
              config: config,
              host: host(),
              onSave: (changed) async {
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

  String held(WidgetTester tester, String key) => tester.widget<TextField>(find.byKey(Key('host-kb-$key'))).controller!.text;
  String note(WidgetTester tester) => tester.widget<Text>(find.byKey(const Key('host-kb-note'))).data!;

  testWidgets('each operation lists who can do it, in the order the host would ask, and says when nothing can', (tester) async {
    await show(tester, const CanoaConfig());
    expect(find.byKey(const Key('assist-embed-offer-openai:work')), findsOneWidget);
    expect(find.byKey(const Key('assist-complete-offer-harness:claude')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('assist-embed-asked'))).data, contains('openai:work'));
    expect(tester.widget<Text>(find.byKey(const Key('assist-rerank-asked'))).data, contains('no backend is set up'));
    expect(find.byKey(const Key('assist-rerank-none')), findsOneWidget);
  });

  testWidgets('a backend is picked from the offers, its models are split by what they report, and where the text goes is said', (tester) async {
    await show(tester, const CanoaConfig());
    await tester.tap(find.byKey(const Key('host-kb-embedder_use-pick')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('OpenAI, account work (openai:work)').last);
    await tester.pumpAndSettle();
    expect(held(tester, 'embedder_use'), 'openai:work');
    expect(
      tester.widget<Text>(find.byKey(const Key('host-kb-embedder_where'))).data,
      allOf(contains('OpenAI, account work'), contains('https://api.openai.com/v1')),
    );
    await tester.tap(find.byKey(const Key('host-kb-embedder_model-pick')));
    await tester.pumpAndSettle();
    expect(find.text('text-embedding-3-small'), findsOneWidget);
    expect(find.byKey(const Key('host-kb-embedder_model-unreported')), findsOneWidget);
    expect(find.text('Other models (capabilities not reported)'), findsOneWidget);
    await tester.tap(find.text('mystery-1'));
    await tester.pumpAndSettle();
    expect(held(tester, 'embedder_model'), 'mystery-1', reason: 'a model that reports nothing may still be chosen');
  });

  testWidgets('a choice is tried with the host, and what it said is shown', (tester) async {
    await show(
      tester,
      CanoaConfig.parse(
        '[host.kb.embedder]\nuse = "openai:work"\nmodel = "text-embedding-3-small"\n[host.kb.ranker]\nuse = "harness:claude"\nmodel = "haiku"\n',
      ),
    );
    await tester.tap(find.byKey(const Key('host-kb-embedder_test')));
    await tester.pumpAndSettle();
    expect(tester.widget<Text>(find.byKey(const Key('host-kb-embedder_test-result'))).data, allOf(contains('1536'), contains('120')));
    await tester.tap(find.byKey(const Key('host-kb-ranker_test')));
    await tester.pumpAndSettle();
    expect(tester.widget<Text>(find.byKey(const Key('host-kb-ranker_test-result'))).data, contains('needs a new sign-in'));
    expect(tried, [
      {'role': 'embedder', 'use': 'openai:work', 'model': 'text-embedding-3-small'},
      {'role': 'ranker', 'use': 'harness:claude', 'model': 'haiku'},
    ]);
  });

  testWidgets('the first backend, the ones after it in their order, an agent on its login and the warm sessions reach canoa.cfg', (tester) async {
    await show(tester, const CanoaConfig());
    await tester.enterText(find.byKey(const Key('assist-complete-use')), 'openai:work');
    await tester.tap(find.byKey(const Key('assist-complete-fallback-add')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Claude Code (harness:claude)').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('assist-complete-fallback-harness:claude')), findsOneWidget);
    await tester.tap(find.byKey(const Key('assist-harness-add')));
    await tester.pumpAndSettle();
    expect(tester.widget<FilledButton>(find.byKey(const Key('host-kb-save'))).onPressed, isNull, reason: 'an agent backend needs the agent that runs it');
    await tester.enterText(find.byKey(const Key('assist-harness-0-name')), 'claude');
    await tester.enterText(find.byKey(const Key('assist-harness-0-source')), 'Claude');
    await tester.enterText(find.byKey(const Key('assist-harness-0-model')), 'haiku');
    await tester.enterText(find.byKey(const Key('assist-pool-idle')), '90');
    await tester.enterText(find.byKey(const Key('assist-pool-max')), '1');
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('host-kb-save')));
    await tester.pumpAndSettle();
    final reread = CanoaConfig.parse(saved.single.encode());
    expect(reread.assistOperations['complete']?.use, 'openai:work');
    expect(reread.assistOperations['complete']?.fallback, ['harness:claude']);
    expect(reread.assistHarnesses, {
      'claude': {'source': 'Claude', 'model': 'haiku'},
    });
    expect(reread.assistPool, (idleSeconds: 90, maxSessions: 1));
    expect(note(tester), contains('Saved'));
  });

  testWidgets('a backend asked after another can be moved earlier', (tester) async {
    await show(tester, CanoaConfig.parse('[assist.embed]\nuse = "a"\nfallback = ["b", "c"]\n'));
    await tester.tap(find.byKey(const Key('assist-embed-fallback-c-earlier')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('host-kb-save')));
    await tester.pumpAndSettle();
    expect(CanoaConfig.parse(saved.last.encode()).assistOperations['embed']?.fallback, ['c', 'b']);
  });

  testWidgets('the words are the person\'s language', (tester) async {
    await show(tester, const CanoaConfig(), language: 'es');
    expect(find.text('Buscar por significado'), findsWidgets);
    expect(find.text('Agregar un agente'), findsOneWidget);
    expect(find.text('Add an agent'), findsNothing);
  });
}
