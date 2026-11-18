// What an overlay of a new session needs of this computer is shown in the effective setup beside the layers, from the host's
// answer, with the Store's runtimes offered for a program that is missing.
@TestOn('vm')
library;

import 'dart:convert';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/effective_setup.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/store/overlay_needs.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

const _source = 'https://example.invalid/team.git@main';
const _files = {'overlay.cfg': 'name = "review"\n[runtime]\ncommands = ["git"]\n'};

MemoryOverlays _installed() {
  final overlays = MemoryOverlays();
  final bytes = {for (final MapEntry(:key, :value) in _files.entries) key: utf8.encode(value)};
  overlays.installedFiles['review'] = bytes;
  overlays.records['review'] = InstalledOverlay(
    name: 'review',
    source: _source,
    commit: 'c' * 40,
    digest: '${treeOfFiles(bytes).digest.digest}',
    files: const {},
  );
  return overlays;
}

Future<void> _open(WidgetTester tester, {OverlayNeeds? needs, Future<void> Function(List<String> providers)? onInstall}) async {
  await tester.binding.setSurfaceSize(const Size(900, 1400));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    home: Scaffold(
      body: SingleChildScrollView(
        child: EffectiveSetup(
          config: CanoaConfig.parse('[overlays]\nreview = "$_source"\n'),
          agent: 'Claude',
          session: const Profile(),
          home: MemoryHome(_installed()),
          overlayNeeds: needs,
          onInstallRuntime: onInstall,
        ),
      ),
    ),
  ));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a program an overlay needs and this computer lacks is listed, with the Store to install it from', (tester) async {
    final asked = <List<String>>[];
    await _open(
      tester,
      needs: (name) async => name == 'review'
          ? [
              const NeedLine(key: 'runtime.commands: git', met: true, how: 'found /usr/bin/git'),
              if (asked.isEmpty)
                const NeedLine(key: 'runtime.python', met: false, how: "Python 3.11 or later is needed; the Store's Python installs it", offer: 'python_store')
              else
                const NeedLine(key: 'runtime.python', met: true, how: 'found /programs/python/bin/python'),
            ]
          : const [],
      onInstall: (providers) async => asked.add(providers),
    );
    expect(find.byKey(const Key('effective-needs-review-runtime.python')), findsOneWidget);
    expect(find.textContaining("overlay:review: needs python: Python 3.11 or later is needed"), findsOneWidget);
    expect(find.byKey(const Key('effective-needs-review-runtime.commands: git')), findsNothing, reason: 'what is met is not a need');
    await tester.tap(find.byKey(const Key('effective-needs-install')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(asked, [['python_store']], reason: 'the runtime the host offers is the one installed');
    expect(find.byKey(const Key('effective-needs-review-runtime.python')), findsNothing, reason: 'the host is asked again once it is installed');
    expect(find.byKey(const Key('effective-needs-install')), findsNothing);
  });

  testWidgets('a need no Store runtime provides offers nothing to install', (tester) async {
    await _open(
      tester,
      needs: (_) async => const [NeedLine(key: 'runtime.commands: canoa-absent', met: false, how: 'canoa-absent is needed and is not on the PATH')],
      onInstall: (_) async {},
    );
    expect(find.byKey(const Key('effective-needs-review-runtime.commands: canoa-absent')), findsOneWidget);
    expect(find.byKey(const Key('effective-needs-install')), findsNothing);
  });

  testWidgets('a host that says nothing of an overlay lists no need for it', (tester) async {
    await _open(tester, onInstall: (_) async {});
    expect(find.textContaining('needs '), findsNothing);
    expect(find.byKey(const Key('effective-needs-install')), findsNothing);
  });
}
