import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('shows Canoa multi-session chat shell', (tester) async {
    final store = SessionStore();
    await tester.pumpWidget(CanoaApp(store: store));
    expect(find.text('Canoa'), findsWidgets);
    expect(find.text('Getting started'), findsWidgets);
    expect(find.textContaining('Preview'), findsWidgets);
    expect(find.byIcon(Icons.add_comment_outlined), findsOneWidget);
    store.dispose();
  });
}
