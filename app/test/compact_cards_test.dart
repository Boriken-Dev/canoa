import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:canoa/store/store_cards.dart';

void main() {
  Widget card(String name, String? summary, {bool on = false}) => CompactCard(
    key: Key('card-$name'),
    leading: const Icon(Icons.extension),
    title: name,
    summary: summary,
    trailing: on ? Switch(value: true, onChanged: (_) {}) : null,
    onOpen: () {},
  );

  for (final (label, width) in [('phone', 390.0), ('tablet', 820.0), ('desktop', 1280.0), ('wide', 1900.0)]) {
    testWidgets('every compact card is one size, with a long name, a long summary, none and a switch, at $label width', (tester) async {
      await tester.binding.setSurfaceSize(Size(width, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final names = ['a', 'a-name-that-is-far-too-long-for-one-line-of-a-card-in-any-grid-of-them', 'plain', 'with-switch'];
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: CardList(
            evenFrom: compactCardWidth,
            cards: [
              card(names[0], null),
              card(names[1], 'A summary that runs on and on past the end of the line, so that it has to be cut short with an ellipsis. ' * 3),
              card(names[2], 'Short.'),
              card(names[3], 'Has its switch', on: true),
            ],
          ),
        ),
      ));
      final sizes = {for (final name in names) tester.getSize(find.byKey(Key('card-$name'))).toString()};
      expect(sizes, hasLength(1), reason: 'the cards are all one size: $sizes');
      expect(tester.getSize(find.byKey(Key('card-${names[0]}'))).height, compactCardHeight + 8, reason: 'the height is the constant, and its margin');
      final summaries = tester.widgetList<Text>(find.byKey(const Key('card-summary')));
      expect(summaries.every((text) => text.maxLines == 1 && text.overflow == TextOverflow.ellipsis), isTrue);
      final columns = {for (final name in names) tester.getTopLeft(find.byKey(Key('card-$name'))).dx}.length;
      expect(columns, width < 700 ? 1 : (width < 1000 ? 2 : (width < 1400 ? 3 : 4)).clamp(1, names.length), reason: 'the grid reflows by width');
    });
  }
}
