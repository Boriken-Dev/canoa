// What a task pane and the app it frames say to each other, and pairing the pane's key with its host by PIN from inside the pane.
import 'dart:convert';

import 'package:canoa/client/pane_message.dart';
import 'package:canoa/client/pane_pairing.dart';
import 'package:canoa/client/relation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

final _host = 'sha256:${'ab' * 32}';

void main() {
  group('the messages', () {
    test('the app\'s two are fixed JSON the pane\'s parser reads', () {
      expect(jsonDecode(readyMessage), {'canoa': 'ready'});
      expect(jsonDecode(signInMessage), {'canoa': 'signin'});
    });

    test('a relation message carries a relation, or none for nothing in front', () {
      final sent = jsonEncode({
        'canoa': 'relation',
        'relation': {'kind': 'conversation', 'key': 'c-1', 'label': 'Quarterly numbers'},
      });
      expect(parseRelationMessage(sent)!.relation, const Relation(kind: Relation.conversationKind, key: 'c-1', label: 'Quarterly numbers'));
      final scoped = parseRelationMessage('{"canoa":"relation","relation":{"kind":"document","key":"d","scope":"s"}}')!;
      expect(scoped.relation, const Relation(kind: Relation.documentKind, key: 'd', scope: 's'));
      final none = parseRelationMessage('{"canoa":"relation","relation":null}');
      expect(none, isNotNull, reason: 'a message that says nothing is in front is still a message');
      expect(none!.relation, isNull);
    });

    test('anything else is no message and changes nothing', () {
      for (final other in <Object?>[
        null,
        3,
        'relation',
        '',
        '{',
        '[]',
        '{"canoa":"ready"}',
        '{"canoa":"relation"}',
        '{"relation":{"kind":"conversation","key":"c"}}',
        '{"canoa":"relation","relation":{"kind":"conversation"}}',
        '{"canoa":"relation","relation":"conversation:c"}',
        '{"canoa":"relation","relation":{"kind":"","key":"k"}}',
        {'canoa': 'relation', 'relation': null},
        '{"canoa":"relation","relation":null,"pad":"${'x' * 5000}"}',
      ]) {
        expect(parseRelationMessage(other), isNull, reason: '$other'.length < 40 ? '$other' : '${'$other'.substring(0, 40)}...');
      }
    });
  });

  group('pairing from the pane', () {
    Future<void> pump(WidgetTester tester, {required PairByPinFake pair, required void Function() onPaired}) async {
      await tester.binding.setSurfaceSize(const Size(380, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: PanePairing(hostPeer: _host, origin: 'https://canoa.example', pair: pair.call, onPaired: onPaired),
          ),
        ),
      ));
    }

    Future<void> enter(WidgetTester tester, String address, String pin) async {
      await tester.enterText(find.byKey(const Key('pane-pairing-address')), address);
      await tester.enterText(find.byKey(const Key('pane-pairing-pin')), pin);
      await tester.tap(find.byKey(const Key('pane-pairing-go')));
      await tester.pumpAndSettle();
    }

    testWidgets('names the command to run on the host with this page\'s origin', (tester) async {
      await pump(tester, pair: PairByPinFake(_host), onPaired: () {});
      expect(find.textContaining('canoa pair --method noise --origin https://canoa.example'), findsOneWidget);
    });

    testWidgets('pairs by the address and PIN, then starts the app over', (tester) async {
      final pair = PairByPinFake(_host);
      var started = 0;
      await pump(tester, pair: pair, onPaired: () => started++);
      await enter(tester, ' wss://desk.example:7801 ', ' 123456 ');
      expect(pair.asked, [(Uri.parse('wss://desk.example:7801'), '123456')]);
      expect(started, 1);
    });

    testWidgets('a pairing with another host than the one that served the page is refused and starts nothing', (tester) async {
      final pair = PairByPinFake('sha256:${'cd' * 32}');
      var started = 0;
      await pump(tester, pair: pair, onPaired: () => started++);
      await enter(tester, 'ws://other.example:7801', '123456');
      expect(started, 0);
      expect(find.textContaining('not the host that served this page'), findsOneWidget);
      expect(tester.widget<TextField>(find.byKey(const Key('pane-pairing-pin'))).controller!.text, isEmpty, reason: 'the PIN is spent');
    });

    testWidgets('a failure is said, the PIN is spent, and nothing starts', (tester) async {
      final pair = PairByPinFake(_host)..fail = 'the host closed the link';
      var started = 0;
      await pump(tester, pair: pair, onPaired: () => started++);
      await enter(tester, 'ws://desk.example:7801', '000000');
      expect(started, 0);
      expect(find.textContaining('Pairing failed'), findsOneWidget);
      expect(find.textContaining('the host closed the link'), findsOneWidget);
      expect(tester.widget<TextField>(find.byKey(const Key('pane-pairing-pin'))).controller!.text, isEmpty);
    });

    testWidgets('an address that is not a WebSocket one, or no PIN, is not tried', (tester) async {
      final pair = PairByPinFake(_host);
      await pump(tester, pair: pair, onPaired: () {});
      for (final (address, pin) in [('https://desk.example', '123456'), ('desk.example:7801', '123456'), ('ws://desk.example:7801', ''), ('', '')]) {
        await enter(tester, address, pin);
        expect(find.byKey(const Key('pane-pairing-error')), findsOneWidget, reason: '$address $pin');
      }
      expect(pair.asked, isEmpty);
    });
  });
}

/// Pairs with a host whose id is [peer], noting what it was asked.
class PairByPinFake {
  PairByPinFake(this.peer);

  final String peer;
  String? fail;
  final asked = <(Uri, String)>[];

  Future<String> call(Uri url, String pin) async {
    asked.add((url, pin));
    final why = fail;
    if (why != null) throw StateError(why);
    return peer;
  }
}
