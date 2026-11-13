// The permission card's answers as a test reaches them: Deny and the primary on the card, the rest in the primary's menu.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final _card = find.byKey(const Key('permission-card'));

Future<void> _openMenu(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('permission-more')));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

/// Whether the card shows [name] as a button, or has it in the menu.
bool onCard(String name) => find.descendant(of: _card, matching: find.text(name)).evaluate().isNotEmpty;

/// Taps the answer called [name], opening the menu for one that is in it.
Future<void> answerWith(WidgetTester tester, String name) async {
  if (!onCard(name)) await _openMenu(tester);
  await tester.tap(find.text(name).last);
  await tester.pump();
}

/// Every answer the card offers, in the order it shows them: Deny, the primary, then the menu's.
Future<List<String>> offered(WidgetTester tester) async {
  String first(Finder from) => (find.descendant(of: from, matching: find.byType(Text)).evaluate().first.widget as Text).data ?? '';
  final names = <String>[
    for (final key in const ['permission-deny', 'permission-primary'])
      if (find.byKey(Key(key)).evaluate().isNotEmpty) first(find.byKey(Key(key))),
  ];
  if (find.byKey(const Key('permission-more')).evaluate().isNotEmpty) {
    await _openMenu(tester);
    final items = find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key as ValueKey<String>).value.startsWith('answer-'));
    for (final item in items.evaluate()) {
      final texts = find.descendant(of: find.byWidget(item.widget), matching: find.byType(Text)).evaluate();
      if (texts.isNotEmpty) names.add((texts.first.widget as Text).data ?? '');
    }
    await tester.tapAt(const Offset(2, 2));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
  }
  return names;
}
