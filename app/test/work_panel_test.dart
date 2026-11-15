// The right and bottom panels: layout toggles and a Layout menu on one state, a launcher and tabs, panels that keep their surfaces and
// sizes while hidden, a surface that keeps its State when moved between panels, and a window that is too narrow for beside.
import 'dart:io' show Directory;

import 'package:canoa/session_store.dart';
import 'package:canoa/shell/layout_controls.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/shell/work_panel.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// The State a probe surface had, in the order they were made.
final _made = <_ProbeState>[];

class _Probe extends WorkSurface {
  const _Probe(this.id, this.title, {this.placements = const [SurfaceSlot.right], this.fails = false, this.shown = true});

  @override
  final String id;
  @override
  final String title;
  @override
  final List<SurfaceSlot> placements;
  final bool fails, shown;

  @override
  IconData get icon => Icons.extension_outlined;

  @override
  bool available(ChatSession session) => shown;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    if (fails) throw StateError('no page');
    return _ProbeView(label: title);
  }
}

class _ProbeView extends StatefulWidget {
  const _ProbeView({required this.label});
  final String label;

  @override
  State<_ProbeView> createState() => _ProbeState();
}

class _ProbeState extends State<_ProbeView> {
  int taps = 0;

  @override
  void initState() {
    super.initState();
    _made.add(this);
  }

  @override
  Widget build(BuildContext context) => Center(
    child: TextButton(key: ValueKey('probe ${widget.label}'), onPressed: () => setState(() => taps++), child: Text('${widget.label} $taps')),
  );
}

class _Rig {
  _Rig(List<WorkSurface> registry, {ShellLayout? layout}) : surfaces = WorkSurfaces(registry), layout = layout ?? ShellLayout() {
    store = SessionStore();
    first = _open('One');
    second = _open('Two');
    shown = ValueNotifier(first);
  }

  final WorkSurfaces surfaces;
  final ShellLayout layout;
  late final SessionStore store;
  late final ChatSession first, second;
  late final ValueNotifier<ChatSession> shown;

  ChatSession _open(String title) => store.addProviderSession(
    title: title, agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-$title',
  )..status = SessionStatus.connected;

  Widget app({bool narrow = false}) => MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(LookId.canoa, Brightness.dark),
    home: Scaffold(
      body: ValueListenableBuilder<ChatSession>(
        valueListenable: shown,
        builder: (context, session, _) => Column(children: [
          Material(
            child: Row(children: [
              const Spacer(),
              LayoutControls(
                layout: layout,
                section: ShellLayout.conversations,
                hasIndex: true,
                hasRight: surfaces.offers(session, SurfaceSlot.right),
                hasBottom: surfaces.offers(session, SurfaceSlot.bottom),
              ),
            ]),
          ),
          Expanded(
            child: PanelFrame(
              layout: layout,
              session: session,
              surfaces: surfaces.of(session.id),
              registry: surfaces,
              narrow: narrow,
              wrap: (panels) => panels,
              child: Material(key: const ValueKey('conversation'), child: Center(child: Text('conversation of ${session.title}'))),
            ),
          ),
        ]),
      ),
    ),
  );

  void dispose() {
    surfaces.dispose();
    store.dispose();
    shown.dispose();
  }
}

Future<_Rig> _show(WidgetTester tester, List<WorkSurface> registry, {Size size = const Size(1200, 800), bool narrow = false, ShellLayout? layout}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  _made.clear();
  final rig = _Rig(registry, layout: layout);
  addTearDown(rig.dispose);
  await tester.pumpWidget(rig.app(narrow: narrow));
  await tester.pump();
  return rig;
}

const _both = [
  _Probe('files', 'Files'),
  _Probe('terminal', 'Terminal', placements: [SurfaceSlot.bottom, SurfaceSlot.right]),
];

Finder _key(String key) => find.byKey(ValueKey(key));

double _rightWidth(WidgetTester tester) => tester.getSize(find.byType(WorkPanel).first).width;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/work_panel_test.dart')));

  group('the controls', () {
    testWidgets('a toggle shows only where its region has a surface for this view', (tester) async {
      await _show(tester, const [_Probe('files', 'Files')]);
      expect(_key('layout-left'), findsOneWidget);
      expect(_key('layout-right'), findsOneWidget);
      expect(_key('layout-bottom'), findsNothing, reason: 'nothing offers the bottom panel');
      await tester.pumpWidget(const SizedBox());
      final none = await _show(tester, const [_Probe('hidden', 'Hidden', shown: false)]);
      expect(_key('layout-right'), findsNothing);
      expect(_key('layout-bottom'), findsNothing);
      expect(none.layout.rightOpen, isFalse);
    });

    testWidgets('each has a tooltip with its shortcut and is pressed while its region shows', (tester) async {
      final rig = await _show(tester, _both);
      expect(find.byTooltip('Toggle section index (Ctrl+B)'), findsOneWidget);
      expect(find.byTooltip('Toggle bottom panel (Ctrl+`)'), findsOneWidget);
      expect(find.byTooltip('Toggle right panel (Ctrl+Alt+B)'), findsOneWidget);
      expect(tester.widget<IconButton>(_key('layout-right')).isSelected, isFalse);
      await tester.tap(_key('layout-right'));
      await tester.pump();
      expect(rig.layout.rightOpen, isTrue);
      expect(tester.widget<IconButton>(_key('layout-right')).isSelected, isTrue);
      expect(tester.widget<IconButton>(_key('layout-left')).isSelected, isTrue);
      await tester.tap(_key('layout-left'));
      await tester.pump();
      expect(rig.layout.collapsed(ShellLayout.conversations), isTrue);
      expect(tester.widget<IconButton>(_key('layout-left')).isSelected, isFalse);
    });

    testWidgets('the Layout menu and the toggles act on one state: Focus main view hides the panels, Restore layout brings them back', (tester) async {
      final rig = await _show(tester, _both);
      await tester.tap(_key('layout-right'));
      await tester.tap(_key('layout-bottom'));
      await tester.pump();
      expect(rig.layout.rightOpen && rig.layout.bottomOpen, isTrue);
      await tester.tap(_key('layout-menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Focus main view'));
      await tester.pumpAndSettle();
      expect(rig.layout.focus, PanelFocus.main);
      expect(find.byType(WorkPanel), findsNothing, reason: 'the panels are out of sight');
      expect(tester.widget<IconButton>(_key('layout-right')).isSelected, isFalse);
      expect(_key('layout-restore'), findsOneWidget);
      await tester.tap(_key('layout-restore'));
      await tester.pumpAndSettle();
      expect(rig.layout.focus, PanelFocus.none);
      expect(find.byType(WorkPanel), findsNWidgets(2));
      expect(tester.widget<IconButton>(_key('layout-right')).isSelected, isTrue);
    });

    testWidgets('Reset layout shows the index, hides both panels and sets every size back', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout
        ..toggleIndex(ShellLayout.conversations)
        ..setRightOpen(true)
        ..setBottomOpen(true)
        ..setRightWidth(500)
        ..setBottomHeight(300);
      await tester.pump();
      await tester.tap(_key('layout-menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Reset layout'));
      await tester.pumpAndSettle();
      expect(rig.layout.collapsed(ShellLayout.conversations), isFalse);
      expect((rig.layout.rightOpen, rig.layout.bottomOpen), (false, false));
      expect((rig.layout.rightWidth, rig.layout.bottomHeight), (ShellLayout.rightDefault, ShellLayout.bottomDefault));
    });

    testWidgets('a toggle pressed while the main view is alone ends that and shows its panel', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout.focusOn(PanelFocus.main);
      await tester.pump();
      await tester.tap(_key('layout-right'));
      await tester.pumpAndSettle();
      expect(rig.layout.focus, PanelFocus.none);
      expect(rig.layout.rightOpen, isTrue);
      expect(find.byType(WorkPanel), findsOneWidget);
    });
  });

  group('the right panel', () {
    testWidgets('empty it offers the surfaces in a launcher, and opening one leaves a tab and a + menu for the rest', (tester) async {
      final rig = await _show(tester, _both);
      await tester.tap(_key('layout-right'));
      await tester.pump();
      expect(find.text('Open a surface'), findsOneWidget);
      expect(_key('launch files'), findsOneWidget);
      expect(_key('panel-add'), findsNothing, reason: 'the launcher is the menu while there is nothing open');
      await tester.tap(_key('launch files'));
      await tester.pump();
      expect(find.text('Open a surface'), findsNothing);
      expect(_key('tab files'), findsOneWidget);
      expect(find.byTooltip('Open a surface'), findsOneWidget);
      await tester.tap(_key('panel-add'));
      await tester.pumpAndSettle();
      await tester.tap(_key('open terminal'));
      await tester.pumpAndSettle();
      expect(rig.surfaces.of(rig.first.id).inSlot(SurfaceSlot.right).map((each) => each.surface.id), ['files', 'terminal']);
      expect(rig.surfaces.of(rig.first.id).activeIn(SurfaceSlot.right)!.surface.id, 'terminal');
    });

    testWidgets('switching tabs keeps every surface as it was, and closing one ends it', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout.setRightOpen(true);
      final mine = rig.surfaces.of(rig.first.id)
        ..openSurface(_both[0])
        ..openSurface(_both[1], SurfaceSlot.right);
      await tester.pump();
      await tester.tap(_key('probe Terminal'));
      await tester.pump();
      expect(find.text('Terminal 1'), findsOneWidget);
      await tester.tap(_key('tab files'));
      await tester.pump();
      expect(find.text('Terminal 1'), findsNothing, reason: 'it is out of sight');
      expect(_made, hasLength(2), reason: 'neither was built again');
      await tester.tap(_key('tab terminal'));
      await tester.pump();
      expect(find.text('Terminal 1'), findsOneWidget);
      await tester.tap(_key('close terminal'));
      await tester.pump();
      expect(mine.find('terminal'), isNull);
      expect(_key('tab terminal'), findsNothing);
      expect(mine.activeIn(SurfaceSlot.right)!.surface.id, 'files');
    });

    testWidgets('hiding it keeps its surfaces and its size, and showing it finds them as they were', (tester) async {
      final rig = await _show(tester, _both);
      rig.surfaces.of(rig.first.id).openSurface(_both[0]);
      rig.layout
        ..setRightOpen(true)
        ..setRightWidth(500);
      await tester.pump();
      expect(_rightWidth(tester), 500);
      await tester.tap(_key('probe Files'));
      await tester.pump();
      final state = _made.single;
      await tester.tap(_key('layout-right'));
      await tester.pump();
      expect(find.byType(WorkPanel), findsNothing, reason: 'hidden');
      expect(rig.layout.rightWidth, 500);
      await tester.tap(_key('layout-right'));
      await tester.pump();
      expect(_rightWidth(tester), 500);
      expect(_made, hasLength(1));
      expect(identical(_made.single, state) && state.taps == 1, isTrue, reason: 'the same State, its count intact');
    });

    testWidgets('it is resized by a drag within 320 and half the window, and reset by a double click', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout.setRightOpen(true);
      await tester.pump();
      expect(_rightWidth(tester), 420);
      final divider = _key('divider Resize the right panel');
      await tester.drag(divider, const Offset(-60, 0));
      await tester.pump();
      expect(_rightWidth(tester), 480);
      await tester.drag(divider, const Offset(-900, 0));
      await tester.pump();
      expect(_rightWidth(tester), 600, reason: 'half of 1200');
      await tester.drag(divider, const Offset(900, 0));
      await tester.pump();
      expect(_rightWidth(tester), ShellLayout.rightMin);
      await tester.tap(divider);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(divider);
      await tester.pump(const Duration(milliseconds: 500));
      expect(_rightWidth(tester), 420);
      expect(rig.layout.rightWidth, ShellLayout.rightDefault);
    });

    testWidgets('Focus fills the window with it and Restore layout returns to the arrangement it left', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout
        ..setRightOpen(true)
        ..setRightWidth(500);
      await tester.pump();
      await tester.tap(_key('panel-focus'));
      await tester.pumpAndSettle();
      expect(rig.layout.focus, PanelFocus.right);
      expect(_rightWidth(tester), 1200);
      expect(find.text('conversation of One'), findsNothing);
      expect(_key('panel-restore'), findsOneWidget);
      await tester.tap(_key('panel-restore'));
      await tester.pumpAndSettle();
      expect(rig.layout.focus, PanelFocus.none);
      expect(_rightWidth(tester), 500);
      expect(find.text('conversation of One'), findsOneWidget);
    });

    testWidgets('a surface that cannot be built says so in its place, and the conversation and the other surface go on', (tester) async {
      final rig = await _show(tester, const [_Probe('files', 'Files'), _Probe('broken', 'Broken', fails: true)]);
      rig.layout.setRightOpen(true);
      rig.surfaces.of(rig.first.id)
        ..openSurface(rig.surfaces.registry[1])
        ..openSurface(rig.surfaces.registry[0]);
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(find.text('conversation of One'), findsOneWidget);
      expect(_key('probe Files'), findsOneWidget);
      await tester.tap(_key('tab broken'));
      await tester.pump();
      expect(find.textContaining('Broken could not be shown'), findsOneWidget);
    });
  });

  group('moving a surface', () {
    testWidgets('between the right and the bottom panel keeps the same State, the same controller and what was typed in it', (tester) async {
      final rig = await _show(tester, _both);
      final mine = rig.surfaces.of(rig.first.id);
      final opened = mine.openSurface(_both[1], SurfaceSlot.right);
      rig.layout.setRightOpen(true);
      await tester.pump();
      await tester.tap(_key('probe Terminal'));
      await tester.tap(_key('probe Terminal'));
      await tester.pump();
      final state = _made.single;
      expect(state.taps, 2);
      await tester.tap(_key('panel-more'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to the bottom panel'));
      await tester.pumpAndSettle();
      expect(opened.slot, SurfaceSlot.bottom);
      expect(rig.layout.bottomOpen, isTrue, reason: 'the panel it went to shows it');
      expect(_made, hasLength(1), reason: 'not built again');
      expect(identical(_made.single, state), isTrue);
      expect(find.text('Terminal 2'), findsOneWidget);
      expect(identical(mine.find('terminal')!.controller, opened.controller), isTrue);
      expect(mine.inSlot(SurfaceSlot.right), isEmpty);
      // And back, from the bottom panel's menu, which comes first in the tree.
      await tester.tap(_key('panel-more').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to the right panel'));
      await tester.pumpAndSettle();
      expect(opened.slot, SurfaceSlot.right);
      expect(_made, hasLength(1));
      expect(find.text('Terminal 2'), findsOneWidget);
    });

    test('a surface is moved only where it may be shown', () {
      final mine = SessionSurfaces();
      mine.openSurface(_both[0]);
      expect(mine.move('files', SurfaceSlot.bottom), isFalse);
      expect(mine.find('files')!.slot, SurfaceSlot.right);
      mine.openSurface(_both[1]);
      expect(mine.find('terminal')!.slot, SurfaceSlot.bottom, reason: 'it opens where its first placement is');
      expect(mine.move('terminal', SurfaceSlot.right), isTrue);
    });
  });

  group('the bottom panel', () {
    testWidgets('has a toggle only once a surface offers it, then a height set by a drag', (tester) async {
      final rig = await _show(tester, _both);
      expect(_key('layout-bottom'), findsOneWidget);
      rig.layout.setBottomOpen(true);
      await tester.pump();
      expect(tester.getSize(find.byType(WorkPanel).first).height, 251, reason: '260 less the divider');
      await tester.drag(_key('divider Resize the bottom panel'), const Offset(0, -60));
      await tester.pump();
      expect(tester.getSize(find.byType(WorkPanel).first).height, 311);
      await tester.pump(const Duration(milliseconds: 500));
      await tester.tap(_key('layout-bottom'));
      await tester.pump();
      expect(find.byType(WorkPanel), findsNothing);
      expect(rig.layout.bottomHeight, 320);
    });
  });

  group('sessions', () {
    testWidgets('each has its own surfaces, restored when it is shown again', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout.setRightOpen(true);
      rig.surfaces.of(rig.first.id).openSurface(_both[0]);
      await tester.pump();
      expect(_key('tab files'), findsOneWidget);
      rig.shown.value = rig.second;
      await tester.pump();
      expect(_key('tab files'), findsNothing);
      expect(find.text('Open a surface'), findsOneWidget, reason: 'the second session has opened nothing');
      expect(find.text('conversation of Two'), findsOneWidget);
      rig.shown.value = rig.first;
      await tester.pump();
      expect(_key('tab files'), findsOneWidget);
      expect(rig.surfaces.of(rig.first.id).open.single.controller, isNotNull);
    });
  });

  group('a narrow window', () {
    testWidgets('shows one panel across it with a way back to the conversation', (tester) async {
      final rig = await _show(tester, _both, size: const Size(390, 800), narrow: true);
      rig.layout.setRightOpen(true);
      rig.surfaces.of(rig.first.id).openSurface(_both[0]);
      await tester.pump();
      expect(_rightWidth(tester), 390);
      expect(find.text('conversation of One'), findsNothing);
      await tester.tap(_key('panel-back'));
      await tester.pumpAndSettle();
      expect(find.byType(WorkPanel), findsNothing);
      expect(find.text('conversation of One'), findsOneWidget);
      expect(_key('probe Files'), findsNothing);
    });
  });

  group('persistence', () {
    test('panel flags and sizes come back, and a main view shown alone does not', () async {
      final backend = MemoryLayoutBackend();
      final first = ShellLayout(backend: backend)
        ..setRightOpen(true)
        ..setRightWidth(512, keep: true)
        ..setBottomHeight(300, keep: true)
        ..focusOn(PanelFocus.main);
      await first.flush();
      final again = ShellLayout(backend: backend);
      await again.load();
      expect((again.rightOpen, again.bottomOpen), (true, false));
      expect((again.rightWidth, again.bottomHeight), (512, 300));
      expect(again.focus, PanelFocus.none);
    });
  });

  group('goldens', () {
    testWidgets('the right panel with its launcher, at desktop width', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout.setRightOpen(true);
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/work_panel_launcher_desktop.png'));
    });

    testWidgets('the right and bottom panels with tabs, at desktop width', (tester) async {
      final rig = await _show(tester, _both);
      rig.layout
        ..setRightOpen(true)
        ..setBottomOpen(true);
      rig.surfaces.of(rig.first.id)
        ..openSurface(_both[0])
        ..openSurface(_both[1], SurfaceSlot.bottom);
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/work_panel_desktop.png'));
    });

    testWidgets('a panel across a phone, with its way back', (tester) async {
      final rig = await _show(tester, _both, size: const Size(390, 800), narrow: true);
      rig.layout.setRightOpen(true);
      rig.surfaces.of(rig.first.id).openSurface(_both[0]);
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/work_panel_phone.png'));
    });
  });
}

Widget debugRig(ShellLayout layout) {
  final rig = _Rig(_both, layout: layout);
  rig.surfaces.of(rig.first.id).openSurface(_both[0]);
  return rig.app();
}
