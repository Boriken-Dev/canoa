// What a new session's composer offers for mode, model and effort before its agent has started, per source: the choices the app
// knows the Claude and Codex presets have, a plain statement for an agent it knows nothing of, labels in a person's words, and the
// choices reaching the agent that starts on the first message.
import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/chat/agent_events.dart';
import 'package:canoa/chat/modes.dart';
import 'package:canoa/chat/session_options.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/source_choices.dart';

SessionStore _store([SessionLedger? ledger]) => SessionStore(includePreview: false, ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));

Future<void> _draft(WidgetTester tester, String source) async {
  await newSession(tester);
  await chooseSource(tester, source);
  await openFolder(tester);
}

/// The text a control's pill shows.
String _pill(WidgetTester tester, String key) {
  final texts = find.descendant(of: find.byKey(Key(key)), matching: find.byType(Text));
  return texts.evaluate().map((element) => (element.widget as Text).data).join();
}

void main() {
  const modes = ['Default', 'Manual', 'Accept edits', 'Auto', 'Plan', 'Bypass'];

  testWidgets('Claude offers every mode, and the models its provider lists with their versions and Fable, in sentence case', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), startAgent: agent.start);
    await _draft(tester, 'Claude');
    expect(await menuOf(tester, 'option-mode'), modes);
    expect(await menuOf(tester, 'option-model'), ['Default (Opus 5.5)', 'Opus 5.5', 'Fable 5.1', 'Sonnet 5.5', 'Haiku 5.5']);
    expect(await menuOf(tester, 'option-effort'), ['Default', 'Low', 'Medium', 'High', 'Extra high', 'Max']);
    expect(_pill(tester, 'option-model'), 'Default (Opus 5.5)');
    expect(agent.starts, isEmpty, reason: 'looking at the choices starts nothing');
    await finish(tester);
  });

  testWidgets('Codex offers its models and their efforts before it starts, and the efforts follow the model', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), startAgent: agent.start);
    await _draft(tester, 'Codex');
    expect(await menuOf(tester, 'option-mode'), modes);
    expect(await menuOf(tester, 'option-model'), ['Default', 'GPT-6 Luna', 'GPT-6 Sol']);
    expect(await menuOf(tester, 'option-effort'), ['Default', 'Low', 'Medium', 'High'], reason: 'the default model, Luna');
    await pick(tester, 'option-model', 'GPT-6 Sol');
    expect(await menuOf(tester, 'option-effort'), ['Default', 'Low', 'High', 'Extra high']);
    await finish(tester);
  });

  testWidgets('an effort the newly chosen model does not take is taken back', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), startAgent: agent.start);
    await _draft(tester, 'Codex');
    await pick(tester, 'option-model', 'GPT-6 Sol');
    await pick(tester, 'option-effort', 'Extra high');
    expect(_pill(tester, 'option-effort'), 'Extra high');
    await pick(tester, 'option-model', 'GPT-6 Luna');
    expect(_pill(tester, 'option-effort'), 'Default');
    await finish(tester);
  });

  testWidgets('the models are asked once for a source and again only when they are old', (tester) async {
    final host = FakeHost();
    await pumpApp(tester, _store(), askChoices: host.call);
    await _draft(tester, 'Claude');
    await newSession(tester);
    await _draft(tester, 'Claude');
    expect(host.asked.where((label) => label == 'Claude'), hasLength(1));
    await finish(tester);
  });

  testWidgets('Antigravity offers its three modes and says the rest comes after it starts: its provider cannot list before', (tester) async {
    await pumpApp(tester, _store());
    await _draft(tester, 'Antigravity');
    expect(await menuOf(tester, 'option-mode'), ['Default', 'Manual', 'Accept edits', 'Plan', 'Bypass']);
    expect(_pill(tester, 'option-model'), 'Model: after start');
    expect(_pill(tester, 'option-effort'), 'Effort: after start');
    await finish(tester);
  });

  testWidgets("Grok says its models and efforts come after it starts, and offers only Canoa's own Bypass of the modes", (tester) async {
    await pumpApp(tester, _store());
    await _draft(tester, 'Grok');
    expect(_pill(tester, 'option-model'), 'Model: after start');
    expect(_pill(tester, 'option-effort'), 'Effort: after start');
    expect(await menuOf(tester, 'option-mode'), ['Default', 'Its other modes: after it has started', 'Bypass']);
    await finish(tester);
  });

  testWidgets('there is no Gemini preset, and Antigravity and Grok are offered', (tester) async {
    await pumpApp(tester, _store());
    await newSession(tester);
    await tester.tap(find.byKey(const Key('new-session-source')));
    await frames(tester);
    expect(find.byKey(const Key('source Gemini')), findsNothing);
    expect(find.byKey(const Key('source Antigravity')), findsWidgets);
    expect(find.byKey(const Key('source Grok')), findsWidgets);
    await tester.tapAt(const Offset(2, 2));
    await frames(tester);
    await finish(tester);
  });

  testWidgets('an agent the app knows nothing of says its choices come after it starts, not a lone Default', (tester) async {
    await pumpApp(tester, _store());
    await _draft(tester, 'Scripted');
    expect(_pill(tester, 'option-model'), 'Model: after start');
    expect(_pill(tester, 'option-effort'), 'Effort: after start');
    final menu = await menuOf(tester, 'option-mode');
    expect(menu, ['Default', 'Its other modes: after it has started', 'Bypass'], reason: "Canoa's own Bypass works with any agent");
    await finish(tester);
  });

  testWidgets("what the agent offered last time shows where the host cannot say, and the host's list wins where it can", (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.rememberSource('Claude', options: OptionsAgentOptions.parsed, images: true, files: true);
    ledger.rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true);
    await pumpApp(tester, _store(ledger));
    await _draft(tester, 'Scripted');
    expect(await menuOf(tester, 'option-model'), ['Default', 'Default model', 'Opus', 'Refused']);
    expect(await menuOf(tester, 'option-effort'), ['Default', 'Low', 'High']);
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    expect(await menuOf(tester, 'option-model'), ['Default (Opus 5.5)', 'Opus 5.5', 'Fable 5.1', 'Sonnet 5.5', 'Haiku 5.5']);
    await finish(tester);
  });

  testWidgets('the mode, model and effort chosen before the first message reach the agent that starts', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final ledger = SessionLedger(MemoryLedgerBackend());
    await pumpApp(tester, _store(ledger), startAgent: agent.start);
    await _draft(tester, 'Claude');
    await pick(tester, 'option-mode', 'Accept edits');
    await pick(tester, 'option-model', 'Opus 5.5');
    await pick(tester, 'option-effort', 'High');
    expect(_pill(tester, 'option-mode'), 'Accept edits');
    expect(agent.starts, isEmpty);
    await send(tester, 'Fix the tests');
    final set = [for (final request in agent.requests) if (request['method'] == 'session/set_config_option') request['params']];
    expect(set.map((params) => (params['configId'], params['value'])), [('mode', 'accept_edits'), ('model', 'opus'), ('effort', 'high')]);
    expect(agent.mode, 'accept_edits');
    await finish(tester);
  });

  testWidgets("Bypass chosen before the start is the agent's own mode, asked for when it starts", (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), startAgent: agent.start);
    await _draft(tester, 'Claude');
    await pick(tester, 'option-mode', 'Bypass');
    await send(tester, 'Go');
    expect(agent.mode, 'bypass');
    await finish(tester);
  });

  test("a preset has its provider's icon, as a hosted entry of the same provider does", () {
    AgentSource preset(String label) => AgentSource.preset(agentPresets.firstWhere((each) => each.label == label));
    expect(sourceIcon(preset('Claude')), providerIcon('claude'));
    expect(sourceIcon(preset('Codex')), providerIcon('codex'));
    expect(sourceIcon(preset('Grok')), providerIcon('grok'));
    expect(sourceIcon(preset('Antigravity')), providerIcon('antigravity'));
    expect(sourceIcon(AgentSource.hosted('review', harness: 'claude')), sourceIcon(preset('Claude')));
    expect(sourceIcon(AgentSource.command(scripted)), Icons.terminal, reason: 'a command a person wrote');
  });

  group('labels', () {
    String label(OptionRole role, String value, String name) => choiceLabel(role, Choice(value, name));

    test('a raw identifier or capitals throughout read as the design names them', () {
      expect(label(OptionRole.mode, 'bypassPermissions', 'BYPASS'), 'Bypass');
      expect(label(OptionRole.mode, 'bypass', 'BYPASS'), 'Bypass');
      expect(label(OptionRole.mode, 'acceptEdits', 'acceptEdits'), 'Accept edits');
      expect(label(OptionRole.mode, 'accept_edits', 'ACCEPT_EDITS'), 'Accept edits');
      expect(label(OptionRole.mode, 'default', 'default'), 'Manual');
      expect(label(OptionRole.mode, 'read_only', 'READ_ONLY'), 'Read only');
      expect(label(OptionRole.effort, 'xhigh', 'xhigh'), 'Extra high');
      expect(label(OptionRole.effort, 'low', 'low'), 'Low');
    });

    test("words an agent chose, and a model's name, stay as they are", () {
      expect(label(OptionRole.mode, 'plan', 'Plan with review'), 'Plan with review');
      expect(label(OptionRole.mode, 'custom', 'Custom: sandbox on'), 'Custom: sandbox on');
      expect(label(OptionRole.model, 'gpt-5-codex', 'gpt-5-codex'), 'gpt-5-codex');
      expect(label(OptionRole.model, 'gpt5', 'GPT-5'), 'GPT-5');
    });

    testWidgets('a remembered mode an agent named in capitals shows in words', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.rememberSource('Scripted', options: [
        const ConfigOption(id: 'mode', label: 'Mode', current: 'ask', category: 'mode', choices: [Choice('ask', 'ASK'), Choice('bypass', 'BYPASS')]),
      ], images: true, files: true);
      await pumpApp(tester, _store(ledger));
      await _draft(tester, 'Scripted');
      expect(await menuOf(tester, 'option-mode'), ['Default', 'Ask', 'Bypass']);
      await finish(tester);
    });
  });
}
