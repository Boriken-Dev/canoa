// Views in the shell: what an agent sends over the session's updates reaches a view that opens in the side panel, a tab of the bottom
// panel or a window, themed from the look; a button's action reaches the agent as a prompt; and an overlay's own views are read,
// refused when they ask for what Canoa does not draw, and drawn under the overlay's name.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/session_store.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/store/contents_store.dart';
import 'package:canoa/store/entry_details.dart';
import 'package:canoa/store/overlay_contents.dart';
import 'package:canoa/surfaces/a2ui/catalog.dart';
import 'package:canoa/surfaces/a2ui/limits.dart';
import 'package:canoa/surfaces/a2ui/overlay_views.dart';
import 'package:canoa/surfaces/a2ui/views.dart';
import 'package:canoa/theme/look.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';

Map<String, Object?> msg(String key, Map<String, Object?> body) => {'version': '1.0', key: body};

/// A report: a card with a title and a button.
List<Object?> report(String id, {Map<String, Object?>? target}) => [
  msg('createSurface', {'surfaceId': id}),
  msg('updateDataModel', {'surfaceId': id, 'value': {'who': 'Ada'}}),
  msg('updateComponents', {
    'surfaceId': id,
    'components': [
      {'id': 'root', 'component': 'Card', 'child': 'col'},
      {'id': 'col', 'component': 'Column', 'children': ['t', 'b']},
      {'id': 't', 'component': 'Text', 'text': {'@path': '/who'}},
      {'id': 'label', 'component': 'Text', 'text': 'Approve'},
      {'id': 'b', 'component': 'Button', 'child': 'label', 'variant': 'primary', 'action': {'event': {'name': 'approve', 'context': {'by': {'@path': '/who'}}}}},
    ],
  }),
];

Map<String, Object?> update(Map<String, Object?> meta) => {
  'jsonrpc': '2.0',
  'method': 'session/update',
  'params': {
    'sessionId': 's1',
    'update': {'sessionUpdate': 'a2ui', '_meta': meta},
  },
};

Future<void> settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
  await tester.pump();
}

/// A shell's worth of what places a view: the surfaces, the layout, and a button-less page that places each view the store announces.
Widget shell({required SessionStore store, required WorkSurfaces surfaces, required ShellLayout layout, Brightness brightness = Brightness.dark, LookId look = LookId.canoa}) => MaterialApp(
  theme: themeFor(look, brightness),
  home: Scaffold(
    body: Builder(
      builder: (context) {
        _listen ??= store.a2uiOpened.listen((event) => placeA2uiView(context: context, surfaces: surfaces, layout: layout, session: event.$1, view: event.$2));
        return ListenableBuilder(
          listenable: Listenable.merge([surfaces, store]),
          builder: (context, _) {
            final session = store.selected!;
            final open = surfaces.of(session.id).activeIn(SurfaceSlot.right) ?? surfaces.of(session.id).activeIn(SurfaceSlot.bottom);
            return open == null ? const SizedBox(key: ValueKey('no panel')) : open.surface.build(context, session, open.controller);
          },
        );
      },
    ),
  ),
);

StreamSubscription<Object?>? _listen;

WorkSurfaces surfacesOf(SessionStore store) => WorkSurfaces(const [], added: () => [for (final (session, views, view) in store.a2uiViews()) A2uiViewSurface(session.id, views, view)], addedChanged: store.a2uiChanged);

void main() {
  tearDown(() async {
    await _listen?.cancel();
    _listen = null;
  });

  group('a view an agent draws', () {
    testWidgets('opens in the side panel by default, drawn in the look, and a button tells the agent', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = await tester.runAsync(() => host.start(store)) as ChatSession;
      final layout = ShellLayout();
      final surfaces = surfacesOf(store);
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout));

      host.say(update({'a2ui': report('r1')}));
      await settle(tester);

      final mine = surfaces.of(session.id);
      expect(mine.find('a2ui:r1')!.slot, SurfaceSlot.right);
      expect(layout.rightOpen, isTrue);
      expect(find.text('Ada'), findsOneWidget);
      final card = tester.widget<Card>(find.byType(Card));
      expect(card.shape ?? Theme.of(tester.element(find.byType(Card))).cardTheme.shape, isNotNull, reason: 'the card is the theme\'s own');

      await tester.runAsync(() async {
        await tester.tap(find.byKey(const ValueKey('a2ui button b')));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      final prompt = host.sentOf('session/prompt').single['params']['prompt'] as List;
      final text = (prompt.first as Map)['text'] as String;
      expect(text, startsWith('A2UI action: approve'));
      final sent = jsonDecode(text.split('```json\n')[1].split('\n```').first) as Map;
      expect(sent['version'], '1.0');
      expect((sent['action'] as Map)['name'], 'approve');
      expect((sent['action'] as Map)['surfaceId'], 'r1');
      expect((sent['action'] as Map)['sourceComponentId'], 'b');
      expect((sent['action'] as Map)['context'], {'by': 'Ada'});
    });

    testWidgets('a target of tab opens in the bottom panel', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = await tester.runAsync(() => host.start(store)) as ChatSession;
      final layout = ShellLayout();
      final surfaces = surfacesOf(store);
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout));

      host.say(update({'a2ui': {'target': 'tab', 'title': 'Test report', 'messages': report('t1')}}));
      await settle(tester);

      final open = surfaces.of(session.id).find('a2ui:t1')!;
      expect(open.slot, SurfaceSlot.bottom);
      expect(open.title, 'Test report');
      expect(layout.bottomOpen, isTrue);
      expect(find.text('Ada'), findsOneWidget);
    });

    testWidgets('a target of window opens a window of its own, and closing it leaves the view in the session', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = await tester.runAsync(() => host.start(store)) as ChatSession;
      final layout = ShellLayout();
      final surfaces = surfacesOf(store);
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout));

      host.say(update({'a2ui': {'target': 'window', 'title': 'Plan', 'messages': report('w1')}}));
      await settle(tester);
      await tester.pump();

      expect(find.byKey(const ValueKey('a2ui window w1')), findsOneWidget);
      expect(find.text('Plan'), findsOneWidget);
      expect(find.descendant(of: find.byKey(const ValueKey('a2ui window w1')), matching: find.text('Ada')), findsOneWidget);
      expect(surfaces.of(session.id).find('a2ui:w1'), isNull, reason: 'a window is not a panel tab');
      await tester.tap(find.byKey(const ValueKey('a2ui window close')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('a2ui window w1')), findsNothing);
      expect(store.a2uiViewsOf(session)!.views.containsKey('w1'), isTrue);
    });

    testWidgets('the same view is drawn in each look\'s colours', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      await tester.runAsync(() => host.start(store));
      final layout = ShellLayout();
      final surfaces = surfacesOf(store);
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout));
      host.say(update({'a2ui': [
        ...report('r1'),
        msg('updateComponents', {'surfaceId': 'r1', 'components': [
          {'id': 'col', 'component': 'Column', 'children': ['t', 'b', 'p']},
          {'id': 'p', 'component': 'Progress', 'value': 0.4},
        ]}),
      ]}));
      await settle(tester);
      final canoa = tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).backgroundColor;
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout, look: LookId.terminal, brightness: Brightness.light));
      await tester.pump(const Duration(seconds: 1));
      expect(tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).backgroundColor, isNot(canoa));
    });

    testWidgets('what is wrong with a view is told to the agent as the protocol\'s error, at most the limit of times', (tester) async {
      final host = DelegateHost();
      final store = SessionStore()..a2uiLimits = const A2uiLimits(maxErrorReports: 2);
      addTearDown(store.dispose);
      await tester.runAsync(() => host.start(store));
      final session = store.selected!;
      await tester.pumpWidget(shell(store: store, surfaces: surfacesOf(store), layout: ShellLayout()));
      for (var i = 0; i < 6; i++) {
        host.say(update({'a2ui': msg('updateComponents', {'surfaceId': 'ghost', 'components': []})}));
      }
      host.say(update({'a2ui': msg('createSurface', {'surfaceId': 'bad', 'theme': {}})}));
      await settle(tester);
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
      final prompts = host.sentOf('session/prompt');
      expect(prompts, isNotEmpty);
      final first = ((prompts.first['params']['prompt'] as List).first as Map)['text'] as String;
      expect(first, startsWith('A2UI error:'));
      expect(first, contains('"code":"VALIDATION_FAILED"'));
      // Each surface is told of at most the limit (a turn that runs queues the rest), so an agent whose view cannot be drawn does not
      // start a loop: two for the ghost, one for the surface with a theme.
      expect(prompts.length + session.chat.queue.length, 3);
    });
  });

  group('an overlay\'s views', () {
    const catalog = '{"catalogId": "urn:overlay:acme", "components": {"Stat": {"extends": "Metric", "defaults": {"label": "Open issues"}}}}';
    final surface = jsonEncode({
      'title': 'Issues',
      'target': 'tab',
      'messages': [
        msg('createSurface', {'surfaceId': 'main', 'catalogId': 'urn:overlay:acme'}),
        msg('updateDataModel', {'surfaceId': 'main', 'value': {'n': 12}}),
        msg('updateComponents', {'surfaceId': 'main', 'components': [
          {'id': 'root', 'component': 'Stat', 'value': {'@path': '/n'}},
        ]}),
      ],
    });

    test('a view over a catalog of its own is read and drawable', () {
      final read = readOverlayViews([ViewFile('catalogs/acme.json', catalog), ViewFile('surfaces/issues.json', surface)]);
      expect(read.problems, isEmpty);
      expect(read.views.single.problems, isEmpty);
      expect((read.views.single.name, read.views.single.title, read.views.single.target), ('issues', 'Issues', A2uiTarget.tab));
      expect(read.catalogs.single.id, 'urn:overlay:acme');
    });

    test('what asks for a component or a catalog Canoa has not, or for code, is refused with why', () {
      final read = readOverlayViews([
        ViewFile('catalogs/evil.json', '{"catalogId": "urn:overlay:evil", "components": {"Native": {"extends": "WasmModule"}}}'),
        ViewFile('catalogs/broken.json', '{not json'),
        ViewFile('surfaces/unknown_catalog.json', jsonEncode({'messages': [msg('createSurface', {'surfaceId': 'a', 'catalogId': 'urn:nobody'})]})),
        ViewFile('surfaces/unknown_component.json', jsonEncode({'messages': [
          msg('createSurface', {'surfaceId': 'a'}),
          msg('updateComponents', {'surfaceId': 'a', 'components': [{'id': 'root', 'component': 'WebView', 'url': 'https://x.example'}]}),
        ]})),
        ViewFile('surfaces/themed.json', jsonEncode({'messages': [msg('createSurface', {'surfaceId': 'a', 'theme': {'primaryColor': '#f00'}})]})),
        ViewFile('surfaces/no_root.json', jsonEncode({'messages': [msg('createSurface', {'surfaceId': 'a'})]})),
        ViewFile('surfaces/not_a_view.json', '[1,2]'),
      ]);
      expect(read.catalogs, isEmpty);
      expect(read.problems.join(), allOf(contains('Native extends'), contains('not JSON')));
      final by = {for (final view in read.views) view.name: view};
      expect(by['unknown_catalog']!.problems.join(), contains('No catalog "urn:nobody"'));
      expect(by['unknown_component']!.problems.join(), contains('"WebView" is not a component'));
      expect(by['themed']!.problems.join(), contains('theme'));
      expect(by['no_root']!.problems.join(), contains('root'));
      expect(by['not_a_view']!.problems, isNotEmpty);
      expect(read.views.every((each) => !each.drawable), isTrue);
    });

    testWidgets('an overlay\'s view opens in the panel it asks for, under the overlay\'s name, drawn with the theme', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = await tester.runAsync(() => host.start(store)) as ChatSession;
      final layout = ShellLayout();
      final surfaces = surfacesOf(store);
      await tester.pumpWidget(shell(store: store, surfaces: surfaces, layout: layout));
      final read = readOverlayViews([ViewFile('catalogs/acme.json', catalog), ViewFile('surfaces/issues.json', surface)]);

      expect(store.showOverlayView(session, 'acme', read.views.single, read), isEmpty);
      await tester.pump();

      final open = surfaces.of(session.id).find('a2ui:overlay:acme:main')!;
      expect((open.slot, open.title), (SurfaceSlot.bottom, 'Issues'));
      expect(find.text('Open issues'), findsOneWidget, reason: 'the alias Stat is a Metric with its label preset');
      expect(find.text('12'), findsOneWidget);
      expect(host.sentOf('session/prompt'), isEmpty, reason: 'an overlay\'s view is not the agent\'s: nothing is told to it');

      // Shown again, it starts over and opens again.
      expect(store.showOverlayView(session, 'acme', read.views.single, read), isEmpty);
    });

    testWidgets('a view that cannot be drawn is not opened', (tester) async {
      final host = DelegateHost();
      final store = SessionStore();
      addTearDown(store.dispose);
      final session = await tester.runAsync(() => host.start(store)) as ChatSession;
      final read = readOverlayViews([ViewFile('surfaces/bad.json', jsonEncode({'messages': [msg('createSurface', {'surfaceId': 'a', 'catalogId': 'urn:nobody'})]}))]);
      expect(store.showOverlayView(session, 'acme', read.views.single, read), isNotEmpty);
      expect(store.a2uiViewsOf(session), isNull);
    });

    testWidgets('the overlay page lists its views, opens a drawable one, and shows why another cannot be', (tester) async {
      final opened = <String>[];
      final contents = OverlayContents(views: [
        ViewFile('catalogs/acme.json', catalog),
        ViewFile('surfaces/issues.json', surface),
        ViewFile('surfaces/bad.json', jsonEncode({'messages': [msg('createSurface', {'surfaceId': 'a', 'catalogId': 'urn:nobody'})]})),
      ]);
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: EntryFactsView(
              facts: null,
              overlay: 'acme',
              read: EntryContents(key: 'k', readAt: DateTime(2026), contents: contents),
              onOpenView: (overlay, view, brought) {
                opened.add('$overlay/${view.name}');
                return const [];
              },
            ),
          ),
        ),
      ));
      expect(find.byKey(const Key('details-views')), findsOneWidget);
      expect(find.byKey(const Key('details-view-problem-bad')), findsOneWidget);
      expect(find.byKey(const Key('details-view-open-bad')), findsNothing);
      await tester.tap(find.byKey(const Key('details-view-open-issues')));
      expect(opened, ['acme/issues']);
    });

    test('the host\'s answer carries the views and the app reads them back', () {
      final contents = OverlayContents.fromJson({'views': [{'path': 'surfaces/a.json', 'text': '{}'}, {'path': 1}]});
      expect(contents.views.single.path, 'surfaces/a.json');
      expect(contents.isEmpty, isFalse);
      expect(OverlayContents.fromJson(contents.toJson()).views.single.text, '{}');
    });
  });

  test('Canoa\'s two catalog ids are the ones a surface may name, and an unknown one is not guessed', () {
    final catalogs = A2uiCatalogs();
    expect(catalogs.ids, [basicCatalogId, canoaCatalogId]);
    expect(catalogs['https://example.com/other.json'], isNull);
  });
}
