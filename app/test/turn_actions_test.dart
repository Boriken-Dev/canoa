// Going back to before one of the person's messages, and editing it to send again: which message is which turn, the buttons on a
// message and what they hand the app, the box an edit is written in, and the words in both languages.
import 'package:canoa/chat/turn_actions.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

ChatSession _conversation() {
  final store = SessionStore();
  final session = store.createDraft();
  session.chat
    ..addMessage(MessageRole.user, 'Fix the failing build')
    ..addMessage(MessageRole.agent, 'Done.')
    ..addMessage(MessageRole.user, 'Now add a test')
    ..addMessage(MessageRole.agent, 'Added.')
    ..addMessage(MessageRole.user, 'Rename the helper');
  return session;
}

List<ChatEntryData> _entries(ChatSession session) => [
  for (final message in session.messages) ModelEntry(message),
];

Future<void> _show(
  WidgetTester tester,
  ChatSession session,
  TurnActions? actions, {
  String language = 'en',
  Size? size,
}) async {
  if (size != null) {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
  }
  await tester.pumpWidget(
    AppTextScope(
      text: AppText(language: language),
      child: MaterialApp(
        home: Scaffold(
          body: ChatTranscript(
            entries: _entries(session),
            renderers: turnRenderers(session, actions),
          ),
        ),
      ),
    ),
  );
}

void main() {
  test('a message is the turn it is among the person\'s own, and nothing else is a turn', () {
    final session = _conversation();
    final entries = _entries(session);
    expect(
      [for (final entry in entries) turnOf(session, entry)],
      [0, null, 1, null, 2],
    );
    expect(
      turnOf(
        session,
        ChatEntry(MessageRole.user, 'Not in it', at: DateTime(2026)),
      ),
      isNull,
    );
  });

  testWidgets(
    'each message of the person has its buttons, and going back hands over its turn and its words',
    (tester) async {
      final session = _conversation();
      final back = <(int, String)>[];
      await _show(
        tester,
        session,
        TurnActions(
          rewind: (_, turn, text) async {
            back.add((turn, text));
            return true;
          },
          resend: (_, _, _, _) async => true,
        ),
      );
      expect(find.byKey(const Key('turn-rewind')), findsNWidgets(3));
      expect(find.byKey(const Key('turn-edit-start')), findsNWidgets(3));
      await tester.tap(find.byKey(const Key('turn-rewind')).at(1));
      await tester.pump();
      expect(back, [(1, 'Now add a test')]);
    },
  );

  testWidgets(
    'without the means to go back the messages are the package\'s own',
    (tester) async {
      await _show(tester, _conversation(), null);
      expect(find.byKey(const Key('turn-rewind')), findsNothing);
      expect(find.text('Fix the failing build'), findsOneWidget);
    },
  );

  testWidgets(
    'an edit is written in the message\'s own place, sent as written and closed once it went',
    (tester) async {
      final session = _conversation();
      final sent = <(int, String, String)>[];
      var goes = false;
      await _show(
        tester,
        session,
        TurnActions(
          rewind: (_, _, _) async => true,
          resend: (_, turn, text, edited) async {
            sent.add((turn, text, edited));
            return goes;
          },
        ),
      );
      await tester.tap(find.byKey(const Key('turn-edit-start')).at(2));
      await tester.pump();
      final box = find.byKey(const Key('turn-edit'));
      expect(
        tester.widget<TextField>(box).controller!.text,
        'Rename the helper',
        reason: 'it starts as it was said',
      );
      expect(
        find.byKey(const Key('turn-rewind')),
        findsNWidgets(2),
        reason: 'the other two are as they were',
      );

      await tester.enterText(box, '   ');
      await tester.tap(find.byKey(const Key('turn-edit-send')));
      await tester.pump();
      expect(sent, isEmpty, reason: 'an empty message is not sent');

      await tester.enterText(box, ' Rename the helper to `load` ');
      await tester.tap(find.byKey(const Key('turn-edit-send')));
      await tester.pumpAndSettle();
      expect(sent, [(2, 'Rename the helper', 'Rename the helper to `load`')]);
      expect(
        box,
        findsOneWidget,
        reason: 'it could not go back, so what was written is still there to try again',
      );

      goes = true;
      await tester.tap(find.byKey(const Key('turn-edit-send')));
      await tester.pumpAndSettle();
      expect(sent, hasLength(2));
      expect(box, findsNothing);

      await tester.tap(find.byKey(const Key('turn-edit-start')).at(0));
      await tester.pump();
      await tester.tap(find.byKey(const Key('turn-edit-cancel')));
      await tester.pump();
      expect(box, findsNothing);
      expect(sent, hasLength(2), reason: 'cancelling sends nothing');
    },
  );

  testWidgets('the buttons say what they do in the person\'s language', (
    tester,
  ) async {
    final actions = TurnActions(
      rewind: (_, _, _) async => true,
      resend: (_, _, _, _) async => true,
    );
    await _show(tester, _conversation(), actions);
    expect(find.byTooltip('Go back to before this message'), findsNWidgets(3));
    expect(find.byTooltip('Edit and resend'), findsNWidgets(3));
    await _show(tester, _conversation(), actions, language: 'es');
    expect(find.byTooltip('Volver a antes de este mensaje'), findsNWidgets(3));
    expect(find.byTooltip('Editar y reenviar'), findsNWidgets(3));
    expect(find.byTooltip('Copiar el mensaje'), findsNWidgets(3));
  });

  for (final (label, size) in [
    ('phone', const Size(390, 700)),
    ('desktop', const Size(1000, 600)),
  ]) {
    testWidgets('the buttons and the edit box draw at $label width', (
      tester,
    ) async {
      final session = _conversation();
      final actions = TurnActions(
        rewind: (_, _, _) async => true,
        resend: (_, _, _, _) async => true,
      );
      await _show(tester, session, actions, size: size);
      expect(tester.takeException(), isNull);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/turn_actions_$label.png'),
      );
      await tester.tap(find.byKey(const Key('turn-edit-start')).at(1));
      await tester.pump();
      expect(tester.takeException(), isNull);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/turn_actions_edit_$label.png'),
      );
    });
  }
}
