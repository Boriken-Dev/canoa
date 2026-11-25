// The start screen of a new session is no tab: with no session open the window shows it alone, with sessions open it shows it in
// the content area under their tabs, and it becomes a tab only when its first message starts the session.
import 'package:canoa/session_store.dart';
import 'package:canoa/tabs/session_tab_strip.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

Finder get _tabs => find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key! as ValueKey<String>).value.startsWith('tab-open-'));

void main() {
  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('at $label width the start screen has no tab and no strip, and a session started from it becomes a tab', (tester) async {
      final agent = OptionsAgent();
      final store = SessionStore();
      await pumpApp(tester, store, linked: agent.openLink, size: size);

      // Nothing is open: the start screen alone.
      expect(store.selected!.isDraft, isTrue);
      expect(find.byType(SessionTabStrip), findsNothing);
      expect(find.byTooltip('Close conversation'), findsNothing);

      await chooseSource(tester, 'Scripted');
      await send(tester, 'First');
      final first = store.selected!;
      await until(tester, () => !first.isDraft, 'the first session to start');
      expect(find.byType(SessionTabStrip), findsNothing, reason: 'one tab alone draws no strip');

      // A new session beside it: the start screen in the content area, not a tab.
      await newSession(tester);
      final start = store.selected!;
      expect(start.isDraft, isTrue);
      expect(find.byType(SessionTabStrip), findsOneWidget, reason: 'the open session stays one tap away');
      expect(_tabs, findsOneWidget);
      expect(find.byKey(Key('tab-open-${first.id}')), findsOneWidget);
      expect(find.byKey(Key('tab-open-${start.id}')), findsNothing);
      expect(find.byTooltip('Close conversation'), findsNothing);
      expect(tester.widget<SessionTabStrip>(find.byType(SessionTabStrip)).current, isNull);

      // Its first message makes it a tab.
      await chooseSource(tester, 'Scripted');
      await send(tester, 'Second');
      await until(tester, () => !start.isDraft, 'the second session to start');
      expect(find.byKey(Key('tab-open-${start.id}')), findsOneWidget);
      expect(_tabs, findsNWidgets(2));
      expect(tester.widget<SessionTabStrip>(find.byType(SessionTabStrip)).current, start.id);

      // Closing the tabs leaves the start screen, with no strip.
      await tester.tap(find.byKey(Key('tab-close-${start.id}')));
      await frames(tester);
      expect(store.selectedId, first.id);
      expect(find.byType(SessionTabStrip), findsNothing, reason: 'the one tab left draws no strip');
      await newSession(tester);
      if (size.width > 600) {
        await tester.tap(find.byKey(Key('tab-close-${first.id}')));
        await frames(tester);
        expect(find.byType(SessionTabStrip), findsNothing);
        expect(store.selected!.isDraft, isTrue);
      }
      await finish(tester);
    });
  }
}
