// The dialog that asks for a sign-in's or an install's inputs is the schema form every other form is: members follow the schema's
// x-order, an x-advanced one is folded, a number is checked as JSON Schema reads it, and what is asked for and not given stops the answer.
import 'package:canoa/store/programs.dart' show ProgramAsk;
import 'package:canoa/store/programs_view.dart' show AskDialog;
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

typedef _Answer = ({Map<String, Object?> values, Map<String, Object?> secrets});

final _schema = {
  'type': 'object',
  'required': ['email'],
  'properties': {
    'issuer': {'type': 'string', 'title': 'Issuer', 'x-advanced': true},
    'port': {'type': 'integer', 'title': 'Port', 'minimum': 1, 'x-order': 1},
    'email': {'type': 'string', 'title': 'Email', 'x-order': 0, 'examples': ['ana@example.org']},
    'region': {'type': 'string', 'title': 'Region', 'default': 'us'},
    'token': {'type': 'string', 'title': 'Token', 'writeOnly': true},
  },
};

Future<Future<_Answer?> Function()> _open(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(800, 1600));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  _Answer? answer;
  var closed = false;
  await tester.pumpWidget(MaterialApp(
    home: Builder(
      builder: (context) => TextButton(
        key: const Key('open'),
        onPressed: () async {
          answer = await showDialog<_Answer>(context: context, builder: (_) => AskDialog(question: ProgramAsk('Sign in', _schema)));
          closed = true;
        },
        child: const Text('open'),
      ),
    ),
  ));
  return () async {
    await tester.pumpAndSettle();
    return closed ? answer : null;
  };
}

void main() {
  testWidgets('it follows the schema: x-order sets the order, an advanced member is folded, and a default is what the box starts with', (tester) async {
    await _open(tester);
    await tester.pump();
    await tester.tap(find.byKey(const Key('open')));
    await tester.pumpAndSettle();
    double top(String key) => tester.getTopLeft(find.byKey(Key('ask-field-$key'))).dy;
    expect(top('email'), lessThan(top('port')), reason: 'x-order 0 before 1');
    expect(top('port'), lessThan(top('region')), reason: 'an explicit order before the rest');
    expect(find.byKey(const Key('ask-field-issuer')), findsNothing, reason: 'an advanced member is under Advanced');
    await tester.tap(find.text('Advanced'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('ask-field-issuer')), findsOneWidget);
    expect(tester.widget<TextField>(find.byKey(const Key('ask-field-region'))).controller!.text, 'us');
  });

  testWidgets('a member asked for and not given, and a number the schema refuses, stop the answer and say why; a secret is answered apart', (tester) async {
    final result = await _open(tester);
    await tester.pump();
    await tester.tap(find.byKey(const Key('open')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('ask-submit')));
    await tester.pump();
    expect(find.text('Email is needed'), findsOneWidget);

    await tester.tap(find.byKey(const Key('ask-suggest-email-ana@example.org')));
    await tester.enterText(find.byKey(const Key('ask-field-port')), '2.5');
    await tester.tap(find.byKey(const Key('ask-submit')));
    await tester.pump();
    expect(find.text('Port is a whole number'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('ask-field-port')), '0');
    await tester.tap(find.byKey(const Key('ask-submit')));
    await tester.pump();
    expect(find.text('Port is at least 1'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('ask-field-port')), '587');
    await tester.enterText(find.byKey(const Key('ask-field-token')), 'hunter2');
    await tester.tap(find.byKey(const Key('ask-submit')));
    final answer = await result();
    expect(answer?.values, {'email': 'ana@example.org', 'port': 587, 'region': 'us'});
    expect(answer?.secrets, {'token': 'hunter2'});
  });
}
