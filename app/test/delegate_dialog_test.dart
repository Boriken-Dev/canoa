// The person's own Delegate task: the host's targets as choices (host, agent, account, model and effort, a folder), the request it
// sends, a refusal that names the choice, and an answer that never came, which is shown as uncertain and never sent again by itself.

import 'package:canoa/acp/delegate_targets.dart';
import 'package:canoa/acp/delegate.dart' show delegateContextMethod;
import 'package:canoa/chat/delegate_dialog.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/delegate_host.dart';

Map<String, Object?> _target(String source, {bool remote = false, List<String> accounts = const [], List<Object?>? models}) => {
  'source': source,
  'title': source.toUpperCase(),
  'host': remote ? 'another machine' : 'this host',
  'remote': remote,
  'accounts': accounts,
  'ready': true,
  'workspace': remote ? {'kind': 'required'} : {'kind': 'folder', 'default': '/work'},
  if (models != null) 'models': {'known': true, 'list': models},
};

const _models = [
  {'id': 'fast', 'name': 'Fast', 'efforts': ['low', 'high']},
  {'id': 'big', 'name': 'Big', 'efforts': ['high']},
];

Future<void> _settle() async {
  await Future<void>.delayed(const Duration(milliseconds: 60));
}

void main() {
  late SessionStore store;
  late DelegateHost host;

  Future<void> open(WidgetTester tester, {Map<String, Object?>? targets}) async {
    await tester.binding.setSurfaceSize(const Size(900, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    store = SessionStore();
    host = DelegateHost();
    if (targets != null) host.targets = targets;
    final session = (await tester.runAsync(() => host.start(store)))!;
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(onPressed: () => showDelegateDialog(context, store, session), child: const Text('open')),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
  }

  final listed = {
    'targets': [
      _target('claude', accounts: ['work', 'home'], models: _models),
      _target('far', remote: true),
    ],
  };

  testWidgets('it asks the host what it may hand a task to and sends the choices as one request', (tester) async {
    await open(tester, targets: listed);
    expect(find.byKey(const Key('delegate host')), findsOneWidget, reason: 'both this host and another machine are offered');
    await tester.enterText(find.byKey(const Key('delegate task field')), 'Run the lint');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegate account')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('work').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delegate model')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Fast').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delegate effort')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('high').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delegate send')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    final sent = host.sentOf('_canoa/delegate/start').single['params'] as Map<String, dynamic>;
    expect(sent['source'], 'claude');
    expect(sent['task'], 'Run the lint');
    expect([sent['account'], sent['model'], sent['effort']], ['work', 'fast', 'high']);
    expect(sent.containsKey('workspace'), isFalse);
    expect(find.byKey(const Key('delegate dialog')), findsNothing, reason: 'the task started, so the dialog is done');
  });

  testWidgets('an agent on another machine needs a folder there and takes no model or account', (tester) async {
    await open(tester, targets: listed);
    await tester.tap(find.byKey(const Key('delegate host')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Another machine').last);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('delegate model')), findsNothing);
    expect(find.byKey(const Key('delegate account')), findsNothing);
    await tester.enterText(find.byKey(const Key('delegate task field')), 'Build it');
    await tester.pump();
    expect(tester.widget<FilledButton>(find.byKey(const Key('delegate send'))).onPressed, isNull, reason: 'no folder there yet');
    await tester.enterText(find.byKey(const Key('delegate folder')), '/srv/project');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegate send')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    final sent = host.sentOf('_canoa/delegate/start').single['params'] as Map<String, dynamic>;
    expect(sent['workspace'], '/srv/project');
    expect(sent.containsKey('model'), isFalse);
  });

  testWidgets('a refusal names the choice and the dialog stays for another try', (tester) async {
    await open(tester, targets: listed);
    host.refuseStart = 'model "big" is not one claude offers';
    await tester.enterText(find.byKey(const Key('delegate task field')), 'x');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegate send')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('delegate refused')), findsOneWidget);
    expect(find.textContaining('"big"'), findsOneWidget);
    expect(find.byKey(const Key('delegate dialog')), findsOneWidget);
  });

  testWidgets('no answer is shown as uncertain, is not sent again by itself, and sending again repeats the same request', (tester) async {
    await open(tester, targets: listed);
    host.silent = true;
    await tester.enterText(find.byKey(const Key('delegate task field')), 'x');
    await tester.pump();
    // The wire waits a minute for an answer; the host's answer to a start is not simulated, so the outcome is what the store gives.
    final session = store.selected!;
    final request = DelegateRequest(requestId: 'r-fixed', source: 'claude', task: 'x');
    final first = tester.runAsync(() => store.delegateTask(session, request).timeout(const Duration(milliseconds: 200), onTimeout: () => const DelegateUncertain()));
    expect(await first, isA<DelegateUncertain>());
    await tester.runAsync(_settle);
    expect(host.sentOf('_canoa/delegate/start'), hasLength(1), reason: 'nothing was sent again by itself');
    // The person sends it again, with the id it had: the host finds its task and makes none.
    host.silent = false;
    final a = await tester.runAsync(() => store.delegateTask(session, request));
    final b = await tester.runAsync(() => store.delegateTask(session, request));
    expect((a! as DelegateStarted).taskId, (b! as DelegateStarted).taskId);
    expect((b as DelegateStarted).repeated, isTrue);
    expect(host.started, hasLength(1));
  });

  testWidgets('a profile and its loadout are chosen among what the agent offers and sent by name', (tester) async {
    await open(tester, targets: {
      'targets': [
        {
          ..._target('claude', models: _models),
          'profiles': [
            {'name': 'review', 'loadouts': ['strict', 'quick']},
            {'name': 'plain', 'loadouts': <String>[]},
          ],
        },
      ],
    });
    expect(find.byKey(const Key('delegate loadout')), findsNothing, reason: 'no profile chosen yet');
    await tester.enterText(find.byKey(const Key('delegate task field')), 'Review it');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegate profile')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('review').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delegate loadout')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('strict').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delegate send')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    final sent = host.sentOf('_canoa/delegate/start').single['params'] as Map<String, dynamic>;
    expect([sent['profile'], sent['loadout']], ['review', 'strict']);
  });

  testWidgets('what the agent is given besides the task is shown in full first, and the task is sent with that preview', (tester) async {
    await open(tester, targets: {'targets': [_target('claude', models: _models)]});
    final held = {
      'contextId': 'preview-1',
      'kind': 'messages',
      'text': 'The conversation of the agent, its most recent turns:\n\n[person]\nFix the build',
      'items': [
        {'name': 'person', 'bytes': 13},
      ],
      'skipped': <String>[],
      'truncated': false,
    };
    host.contexts['messages'] = held;
    await tester.enterText(find.byKey(const Key('delegate task field')), 'Carry on');
    await tester.pump();
    expect(find.byKey(const Key('delegate context preview')), findsNothing);
    await tester.tap(find.byKey(const Key('delegate context')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('The recent turns of this conversation').last);
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('delegate context preview')), findsOneWidget);
    expect(find.textContaining('person  (13 bytes)'), findsOneWidget);
    await tester.tap(find.byKey(const Key('delegate context text')));
    await tester.pumpAndSettle();
    expect(find.textContaining('[person]\nFix the build'), findsOneWidget, reason: 'the whole text is there to read');
    await tester.tap(find.byKey(const Key('delegate send')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    final asked = host.sentOf(delegateContextMethod).single['params'] as Map<String, dynamic>;
    expect(asked['kind'], 'messages');
    final sent = host.sentOf('_canoa/delegate/start').single['params'] as Map<String, dynamic>;
    expect([sent['context'], sent['contextId']], ['messages', 'preview-1']);
  });

  testWidgets('with nothing to hand over the host says so and no task goes out until the person chooses the task text only', (tester) async {
    await open(tester, targets: {'targets': [_target('claude', models: _models)]});
    await tester.enterText(find.byKey(const Key('delegate task field')), 'Carry on');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegate context')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('The files this conversation touched').last);
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('delegate context none')), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const Key('delegate send'))).onPressed, isNull);
    await tester.tap(find.byKey(const Key('delegate context')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Nothing: the task text only').last);
    await tester.pumpAndSettle();
    expect(tester.widget<FilledButton>(find.byKey(const Key('delegate send'))).onPressed, isNotNull);
  });

  testWidgets('the delegation setup shows what the host has and sends the whole setup the person made', (tester) async {
    await open(tester, targets: {
      'targets': [_target('claude', models: _models)],
      'sources': ['claude'],
      'default': 'claude',
      'maxRunning': 2,
      'context': ['messages'],
      'offered': [
        {'source': 'claude', 'title': 'CLAUDE'},
        {'source': 'codex', 'title': 'CODEX'},
      ],
    });
    await tester.tap(find.byKey(const Key('delegate setup open')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('delegation setup')), findsOneWidget);
    expect(tester.widget<Switch>(find.descendant(of: find.byKey(const Key('delegation setup every')), matching: find.byType(Switch))).value, isFalse);
    expect(tester.widget<Checkbox>(find.descendant(of: find.byKey(const Key('delegation setup agent claude')), matching: find.byType(Checkbox))).value, isTrue);
    expect(tester.widget<Checkbox>(find.descendant(of: find.byKey(const Key('delegation setup agent codex')), matching: find.byType(Checkbox))).value, isFalse);
    expect(tester.widget<Checkbox>(find.descendant(of: find.byKey(const Key('delegation setup messages')), matching: find.byType(Checkbox))).value, isTrue);
    await tester.tap(find.byKey(const Key('delegation setup agent codex')));
    await tester.tap(find.byKey(const Key('delegation setup files')));
    await tester.enterText(find.byKey(const Key('delegation setup limit')), '3');
    await tester.pump();
    await tester.tap(find.byKey(const Key('delegation setup save')));
    await tester.runAsync(_settle);
    await tester.pumpAndSettle();
    final sent = host.sentOf('_canoa/delegate/setup').single['params'] as Map<String, dynamic>;
    expect(sent['sources'], ['claude', 'codex']);
    expect(sent['default'], 'claude');
    expect(sent['maxRunning'], 3);
    expect(sent['context'], ['messages', 'files']);
    expect(find.byKey(const Key('delegation setup')), findsNothing, reason: 'the host took it, so the setup is done');
  });

  testWidgets('a host that lists nothing says so', (tester) async {
    await open(tester, targets: {'targets': <Object?>[]});
    expect(find.byKey(const Key('delegate none')), findsOneWidget);
  });
}
