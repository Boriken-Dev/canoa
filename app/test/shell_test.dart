// The shell: a section selector, the selected section's index and its main view; Ctrl+B and a button hide only the index, the
// divider is dragged or moved with the keys, the layout comes back after a restart, and a narrow window has a drawer.
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/shell/shell.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _toggle = ValueKey('layout-left');
final _divider = find.byKey(const ValueKey('divider Resize the section index'));

/// Two sections with an index and one without; the first's main view holds a text field whose text must outlive a visit elsewhere.
Widget _app(ShellLayout layout, {bool narrow = false}) => MaterialApp(
  theme: themeFor(LookId.canoa, Brightness.dark),
  home: AppShell(
    layout: layout,
    narrow: narrow,
    sections: const [
      AppSection(
        id: 'chats',
        label: 'Chats',
        icon: Icons.chat_bubble_outline,
        index: Material(child: Center(child: Text('chats index'))),
        main: Material(child: TextField(key: Key('draft'))),
      ),
      AppSection(
        id: 'tools',
        label: 'Tools',
        icon: Icons.tune,
        index: Material(child: Center(child: Text('tools index'))),
        main: Material(child: Center(child: Text('tools main'))),
      ),
      AppSection(id: 'settings', label: 'Settings', icon: Icons.settings_outlined, main: Material(child: Center(child: Text('settings main'))), atFoot: true),
    ],
  ),
);

Future<void> _show(WidgetTester tester, ShellLayout layout, {Size size = const Size(1100, 700), bool narrow = false}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(_app(layout, narrow: narrow));
  await tester.pump();
}

double _indexWidth(WidgetTester tester) => tester.getSize(find.ancestor(of: find.text('chats index'), matching: find.byType(SizedBox)).first).width;

Future<void> _ctrlB(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyB);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pump();
}

void main() {
  group('the selector', () {
    testWidgets('names each section in a tooltip and for a screen reader, with Settings at its foot', (tester) async {
      await _show(tester, ShellLayout());
      for (final name in const ['Chats', 'Tools', 'Settings']) {
        expect(find.byTooltip(name), findsOneWidget);
        expect(find.bySemanticsLabel(name), findsOneWidget);
      }
      final settings = tester.getTopLeft(find.byKey(const ValueKey('section settings'))).dy;
      expect(settings, greaterThan(tester.getTopLeft(find.byKey(const ValueKey('section tools'))).dy));
      expect(settings, greaterThan(500), reason: 'at the foot');
      expect(tester.getSize(find.byType(SectionRail)).width, SectionRail.width);
    });

    testWidgets('choosing a section shows its index and its main view; one with no index shows only the main view', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout);
      expect(find.text('chats index'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('section tools')));
      await tester.pump();
      expect(find.text('tools index'), findsOneWidget);
      expect(find.text('tools main'), findsOneWidget);
      expect(find.text('chats index'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('section settings')));
      await tester.pump();
      expect(find.text('settings main'), findsOneWidget);
      expect(find.textContaining('index'), findsNothing);
      expect(find.byKey(_toggle), findsNothing, reason: 'nothing to hide');
    });

    testWidgets('a section left keeps its main view as it was, a draft included', (tester) async {
      await _show(tester, ShellLayout());
      await tester.enterText(find.byKey(const Key('draft')), 'half a thought');
      await tester.tap(find.byKey(const ValueKey('section tools')));
      await tester.pump();
      expect(find.byKey(const Key('draft')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('section chats')));
      await tester.pump();
      expect(find.text('half a thought'), findsOneWidget);
    });
  });

  group('the index', () {
    testWidgets('Ctrl+B hides only the index and brings it back at its old width', (tester) async {
      final layout = ShellLayout()..setIndexWidth('chats', 340);
      await _show(tester, layout);
      expect(_indexWidth(tester), 340);
      final before = tester.getSize(find.byKey(const Key('draft'))).width;
      await _ctrlB(tester);
      expect(find.text('chats index'), findsNothing);
      expect(find.byType(SectionRail), findsOneWidget, reason: 'the selector stays');
      expect(find.byKey(_toggle), findsOneWidget, reason: 'a way back stays');
      expect(tester.getSize(find.byKey(const Key('draft'))).width, greaterThan(before), reason: 'the main view takes the room');
      await _ctrlB(tester);
      expect(_indexWidth(tester), 340);
    });

    testWidgets('the button does the same, says so in its tooltip and is pressed while the index shows', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout);
      expect(find.byTooltip('Toggle section index (Ctrl+B)'), findsOneWidget);
      expect(tester.widget<IconButton>(find.byKey(_toggle)).isSelected, isTrue);
      await tester.tap(find.byKey(_toggle));
      await tester.pump();
      expect(layout.collapsed('chats'), isTrue);
      expect(find.text('chats index'), findsNothing);
      expect(tester.widget<IconButton>(find.byKey(_toggle)).isSelected, isFalse);
      await tester.tap(find.byKey(_toggle));
      await tester.pump();
      expect(find.text('chats index'), findsOneWidget);
    });

    testWidgets('hiding one section\'s index leaves another\'s alone, and the draft survives the move', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout);
      await tester.enterText(find.byKey(const Key('draft')), 'kept');
      await tester.tap(find.byKey(_toggle));
      await tester.pump();
      expect(find.text('kept'), findsOneWidget, reason: 'the field moved in the tree and kept its text');
      await tester.tap(find.byKey(const ValueKey('section tools')));
      await tester.pump();
      expect(find.text('tools index'), findsOneWidget);
    });

    testWidgets('a drag resizes within 200 and 480 pixels', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout);
      expect(_indexWidth(tester), 260);
      await tester.drag(_divider, const Offset(60, 0));
      await tester.pump();
      expect(_indexWidth(tester), 320);
      await tester.drag(_divider, const Offset(400, 0));
      await tester.pump();
      expect(_indexWidth(tester), ShellLayout.maxIndex);
      await tester.drag(_divider, const Offset(-900, 0));
      await tester.pump();
      expect(_indexWidth(tester), ShellLayout.minIndex);
      await tester.pump(const Duration(milliseconds: 500));
    });

    testWidgets('the arrow keys resize it once it has the focus, Home and a double click reset it, and a screen reader hears a separator', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout);
      expect(find.bySemanticsLabel('Resize the section index'), findsOneWidget);
      await tester.tap(_divider);
      await tester.pump(const Duration(milliseconds: 500));
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.pump();
      expect(_indexWidth(tester), 276);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await tester.pump();
      expect(_indexWidth(tester), 244);
      await tester.sendKeyEvent(LogicalKeyboardKey.home);
      await tester.pump();
      expect(_indexWidth(tester), 260);
      await tester.drag(_divider, const Offset(100, 0));
      await tester.pump();
      await tester.tap(_divider);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(_divider);
      await tester.pump(const Duration(milliseconds: 500));
      expect(_indexWidth(tester), 260, reason: 'a double click goes back to the default');
    });
  });

  group('kept across runs', () {
    testWidgets('the section, each width, the hidden index, the item and the scroll come back', (tester) async {
      final backend = MemoryLayoutBackend();
      final first = ShellLayout(backend: backend)
        ..setIndexWidth('chats', 300, keep: true)
        ..toggleIndex('tools')
        ..setItem('chats', 'local-3')
        ..setScroll('chats', 120)
        ..select('tools');
      await first.flush();
      final again = ShellLayout(backend: backend);
      await again.load();
      expect(again.section, 'tools');
      expect(again.indexWidth('chats', 260), 300);
      expect(again.collapsed('tools'), isTrue);
      expect(again.collapsed('chats'), isFalse);
      expect(again.of('chats').item, 'local-3');
      expect(again.of('chats').scroll, 120);
      await _show(tester, again);
      expect(find.text('tools main'), findsOneWidget);
      expect(find.text('tools index'), findsNothing, reason: 'its index was hidden');
    });

    test('a file that is not a layout leaves the defaults', () async {
      final backend = MemoryLayoutBackend()..kept = '[1, 2';
      final layout = ShellLayout(backend: backend);
      await layout.load();
      expect(layout.section, ShellLayout.conversations);
      expect(layout.indexWidth('x', 260), 260);
    });

    test('a width is held within the bounds however it was written', () async {
      final backend = MemoryLayoutBackend()..kept = '{"sections": {"chats": {"indexWidth": 9000}}}';
      final layout = ShellLayout(backend: backend);
      await layout.load();
      expect(layout.indexWidth('chats', 260), ShellLayout.maxIndex);
    });
  });

  group('a narrow window', () {
    testWidgets('has the selector and the index in a drawer, and shows only the main view', (tester) async {
      final layout = ShellLayout();
      await _show(tester, layout, size: const Size(500, 800), narrow: true);
      expect(find.byType(SectionRail), findsNothing);
      expect(find.text('chats index'), findsNothing);
      tester.state<ScaffoldState>(find.byType(Scaffold)).openDrawer();
      await tester.pumpAndSettle();
      expect(find.byType(SectionRail), findsOneWidget);
      expect(find.text('chats index'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('section settings')));
      await tester.pumpAndSettle();
      expect(find.text('settings main'), findsOneWidget);
      expect(find.byType(SectionRail), findsNothing, reason: 'a section with no index closes the drawer');
    });
  });
}
