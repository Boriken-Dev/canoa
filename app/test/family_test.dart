// The look family apart from brightness: Claude's and ChatGPT's families, the setting that lets the shown conversation's agent pick
// one, and that a background conversation and any page but the conversations do not.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/theme/follow_provider.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';

LookId _shown(WidgetTester tester) =>
    tester.widget<MaterialApp>(find.byType(MaterialApp)).theme!.extension<CanoaLook>()!.id;

void main() {
  group('the family an agent picks', () {
    test('Claude and Codex pick theirs, any other agent and a missing one keep the chosen family', () {
      LookId pick(String? provider, {bool follow = true, bool inConversations = true}) =>
          familyFor(chosen: LookId.terminal, follow: follow, inConversations: inConversations, provider: provider);
      expect(pick('Claude'), LookId.claude);
      expect(pick('claude'), LookId.claude);
      expect(pick('Codex'), LookId.chatgpt);
      expect(pick('Grok'), LookId.terminal);
      expect(pick(null), LookId.terminal);
      expect(pick('Claude', follow: false), LookId.terminal, reason: 'off by default');
      expect(pick('Claude', inConversations: false), LookId.terminal, reason: 'a page that is not the conversations');
    });

    test('the setting is off by default and survives a write and a read', () {
      expect(const CanoaConfig().followProvider, isFalse);
      final on = const CanoaConfig().copyWith(look: LookId.chatgpt, followProvider: true);
      final back = CanoaConfig.parse(on.encode());
      expect((back.look, back.followProvider), (LookId.chatgpt, true));
    });

    test('the four families differ in more than a colour: the shape of a control too', () {
      final radii = {for (final id in LookId.values) id: lookFor(id, Brightness.dark).controlRadius};
      expect(radii[LookId.chatgpt], greaterThan(radii[LookId.canoa]!));
      expect({for (final id in LookId.values) lookFor(id, Brightness.dark).accent}.length, LookId.values.length);
    });
  });

  group('following the provider in the app', () {
    testWidgets('the shown conversation picks the family and a background one does not', (tester) async {
      final store = SessionStore();
      final claude = store.addProviderSession(title: 'One', agent: 'Claude', peerId: 'local:a', remoteSessionId: 'r1');
      final grok = store.addProviderSession(title: 'Two', agent: 'Grok', peerId: 'local:b', remoteSessionId: 'r2');
      final layout = ShellLayout();
      await pumpApp(tester, store, config: const CanoaConfig(look: LookId.terminal, followProvider: true), layout: layout);
      expect(_shown(tester), LookId.terminal, reason: 'Grok is shown: the chosen family');

      store.select(claude.id);
      await tester.pump();
      expect(_shown(tester), LookId.claude);

      // Another conversation's own turn in the background changes nothing.
      store.rename(grok, 'Two, renamed');
      await tester.pump();
      expect(_shown(tester), LookId.claude);

      layout.select('settings');
      await tester.pump();
      expect(_shown(tester), LookId.terminal, reason: 'a page that is not the conversations keeps the fallback');
      layout.select(ShellLayout.conversations);
      await tester.pump();
      expect(_shown(tester), LookId.claude);
    });

    testWidgets('with the setting off the chosen family stays whatever agent is shown', (tester) async {
      final store = SessionStore();
      store.addProviderSession(title: 'One', agent: 'Claude', peerId: 'local:a', remoteSessionId: 'r1');
      await pumpApp(tester, store, config: const CanoaConfig(look: LookId.canoa));
      expect(_shown(tester), LookId.canoa);
    });
  });
}
