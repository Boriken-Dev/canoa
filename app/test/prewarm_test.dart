// What a draft knows before Send, and how its choices go to the agent when it starts: nothing is started before Send, the
// models and modes come from what the source listed or last offered, the mode and model go together with the effort after the
// model, and a program is looked up on PATH once while PATH is unchanged.
import 'dart:io';

import 'package:canoa/acp/local_agent_io.dart' show findProgram, pathLookupsRun;
import 'package:canoa/chat/session_options.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

SessionStore _remembering() => SessionStore(
  ledger: SessionLedger(MemoryLedgerBackend())
    ..rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true),
);

void main() {
  test('a program is looked up on PATH once while PATH is unchanged', () async {
    final program = Platform.isWindows ? 'where' : 'sh';
    final first = await findProgram(program);
    expect(first, isNotNull);
    final before = pathLookupsRun;
    expect(await findProgram(program), first);
    expect(pathLookupsRun, before, reason: 'the second lookup came from what the first found');
  });

  test('a program that lists its modes in its session has them before it starts', () {
    final choices = SourceChoices.parse(
      '{"known":true,"modes":[{"id":"default","name":"Manual"},{"id":"plan"}],"default_mode":"default",'
      '"models":[{"id":"m","name":"M","efforts":["low","high"]}]}',
    )!;
    final mode = choices.options().where((option) => option.category == 'mode').single;
    expect([for (final choice in mode.choices) (choice.value, choice.label)], [('default', 'Manual'), ('plan', 'plan')]);
    expect(choices.options().map((option) => option.id), ['mode', 'model', 'effort']);
  });

  testWidgets('a draft shows what its source last offered, with when, and starts nothing before Send', (tester) async {
    final agent = OptionsAgent();
    final store = _remembering();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await settle(tester);
    final session = store.selected!;
    expect(optionIn(store.optionsOf(session), OptionRole.model)?.choices.map((choice) => choice.value), contains('opus'));
    expect(modeOptionInOptions(store.optionsOf(session)), isTrue, reason: 'its modes are known too');
    expect(store.optionsSeen(session), isNotNull, reason: 'the date it was last offered');
    expect(agent.starts, isEmpty, reason: 'no process or link until Send');
    expect(agent.requests, isEmpty);
    await finish(tester);
  });

  testWidgets('the mode and the model go to the agent together, and the effort once the model has answered', (tester) async {
    final agent = OptionsAgent()..holdChoices = true;
    final store = _remembering();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-effort', 'High');
    await pick(tester, 'option-mode', 'Accept edits');
    await send(tester, 'First');
    List<Object?> sent() => [
      for (final request in agent.requests)
        if (request['method'] == 'session/set_config_option' || request['method'] == 'session/set_mode')
          request['params']['configId'] ?? 'mode',
    ];
    await until(tester, () => sent().length == 2, 'the mode and the model, both unanswered');
    expect(sent(), unorderedEquals(['mode', 'model']), reason: 'the effort waits for the model');
    agent.releaseChoices();
    await until(tester, () => sent().length == 3, 'the effort');
    expect(sent().last, 'effort');
    agent.releaseChoices();
    await until(tester, () => agent.methods.contains('session/prompt'), 'the message');
    expect(agent.model, 'opus');
    expect(agent.effort, 'high');
    expect(agent.mode, 'edits');
    await finish(tester);
  });
}

bool modeOptionInOptions(List<dynamic> options) => options.any((option) => option.category == 'mode');
