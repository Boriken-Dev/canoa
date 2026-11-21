import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/shell/confirm_remove.dart';
import 'package:canoa/shell/index_pane.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('hostWords unwraps ControlFailed, StateError and Exception prefixes', () {
    expect(hostWords(const ControlFailed(-32001, 'permission denied')), 'permission denied');
    expect(hostWords(StateError('host closed')), 'host closed');
    expect(hostWords(Exception('boom')), 'boom');
    expect(hostWords('plain failure'), 'plain failure');
  });

  testWidgets('confirmRemoval returns false on cancel and true on confirm with custom labels', (tester) async {
    bool? result;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () async {
              result = await confirmRemoval(
                context,
                title: 'Remove item?',
                body: 'Details stay.',
                cancelLabel: 'Keep it',
                confirmLabel: 'Remove it',
              );
            },
            child: const Text('Open'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('confirm-remove')), findsOneWidget);
    await tester.tap(find.byKey(const Key('confirm-remove-no')));
    await tester.pumpAndSettle();
    expect(result, isFalse);

    result = null;
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    expect(find.text('Remove it'), findsOneWidget);
    await tester.tap(find.byKey(const Key('confirm-remove-yes')));
    await tester.pumpAndSettle();
    expect(result, isTrue);
  });

  testWidgets('IndexPane and IndexRow lay out header, subheader, rows, selection and footer', (tester) async {
    var tapped = 0;
    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.light),
        home: Scaffold(
          body: SizedBox(
            width: 280,
            child: IndexPane(
              title: 'Items',
              trailing: const Icon(Icons.add),
              aboveDivider: const [Text('Subheader')],
              body: ListView(
                children: [
                  IndexRow(
                    rowKey: const Key('row-a'),
                    leading: const Icon(Icons.star),
                    title: 'Alpha',
                    subtitle: 'First item',
                    selected: true,
                    onTap: () => tapped++,
                    trailing: const [Icon(Icons.check, size: 14)],
                  ),
                ],
              ),
              footer: const Text('Footer'),
            ),
          ),
        ),
      ),
    );

    expect(find.text('Items'), findsOneWidget);
    expect(find.text('Subheader'), findsOneWidget);
    expect(find.text('Alpha'), findsOneWidget);
    expect(find.text('First item'), findsOneWidget);
    expect(find.text('Footer'), findsOneWidget);
    await tester.tap(find.byKey(const Key('row-a')));
    expect(tapped, 1);
  });
}
