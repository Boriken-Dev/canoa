// Loadout against the real host: an overlay installed from a catalog in a local git repository, chosen for a new session, shows as
// applied in the profile report the host sends; left out of the choice, it does not; and a default turned off reaches new
// sessions only. Needs git, CANOA_TEST_HOST (a built canoa with its examples) and CANOA_TEST_LIBRARY (a built canoa_ffi).
@Timeout(Duration(minutes: 5))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/report.dart';
import 'package:canoa/store/store.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart' show embeddedHostMissing;
import 'fixtures/examples.dart';
import 'fixtures/store_fixtures.dart';

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

void main() {
  final problem = !hasGit ? 'git is not on PATH' : embeddedHostMissing() ?? exampleMissing('profile_agent');

  test('an overlay installed from a catalog and chosen for a new session is applied in its profile report; one left out is not', () async {
    final temp = await Directory.systemTemp.createTemp('canoa-loadout-host');
    HostSupervisor? host;
    try {
      final root = temp.path.replaceAll(r'\', '/');
      // A catalog entry names its overlay by a path in the catalog's own repository, which is how a local one is offered.
      final catalog = Repo(root, 'catalog')
        ..write('overlays/review/overlay.cfg', '[about]\nsummary = "Review conventions."\n\n[env]\nREVIEW_MODE = "strict"\n')
        ..write('catalog.toml', '[overlays.review]\nsource = "./overlays/review"\n')
        ..commit('catalog');
      final config = Directory('$root/config')..createSync();
      final agent = exampleBinary('profile_agent')!;
      final base = CanoaConfig.parse('[agents.Profiled]\nprogram = ${jsonEncode(agent)}\n'
          '[catalogs]\nmain = "${catalog.location}@main#catalog.toml"\n');
      await File('${config.path}/canoa.cfg').writeAsString(base.encode());
      // The host library in this process, as in the app; a source of its own with no address leaves it idle, not running.
      host = HostSupervisor(LibraryHostTool(
        CanoaNative.open(path: File(Platform.environment['CANOA_TEST_LIBRARY']!).absolute.path),
        home: '${config.path}/home',
        program: File(Platform.environment['CANOA_TEST_HOST']!).absolute.path,
      ));
      await host.apply(configPath: '${config.path}/canoa.cfg');
      await _until(() => host!.phase == HostPhase.idle, 'the host to start');

      // Install it the way Loadout does: the catalog read, the overlay fetched and shown, then installed on a yes.
      final home = IoProfileHome(config.path);
      final loaded = await loadCatalog(home.overlays, 'main', base.catalogs['main']!, base.store);
      expect(loaded.problem, isNull);
      final entry = StoreListing([loaded]).entries.single.entry;
      final shown = await previewOverlay(home.overlays, installName(entry), installSource(entry), os: home.os, shared: const {},
          catalogPin: entry.pin);
      expect(shown.installable, isTrue);
      final installed = await installPreview(home.overlays, base, shown, catalog: 'main');
      expect(installed.overlays.keys, ['review']);
      expect(installed.defaultOverlays, ['review']);
      await File('${config.path}/canoa.cfg').writeAsString(installed.encode());
      // The host reads the profile when it starts, so it starts again to apply the change.
      await host.apply(configPath: '${config.path}/canoa.cfg');
      await _until(() => host!.phase == HostPhase.idle, 'the host to start again');

      Future<ProfileReport> reportChoosing(Map<String, Object?> meta) async {
        final link = host!.openLink('Profiled');
        final client = VerifiedAgent.fromChild(
          label: 'Profiled', messages: link.messages, send: link.send, close: link.close, initializeMeta: meta,
        );
        try {
          await client.initialize();
          final opened = await client.openSession(cwd: config.path);
          final report = hostProfileOf((opened.raw['_meta'] as Map)[hostProfileMeta], fallback: 'Profiled');
          return report!.report;
        } finally {
          await client.close();
        }
      }

      ProfileItem? env(ProfileReport report) =>
          report.items.where((item) => item.key == 'env.REVIEW_MODE' && item.layer == 'overlay:review').firstOrNull;

      // Chosen: the choice Loadout makes for a session is what the host is told.
      final chosen = await reportChoosing({overlaysMeta: installed.defaultOverlays});
      expect(env(chosen)?.state, ItemState.applied, reason: chosen.summary);

      // A default turned off: a new session starts without it, and the report says it was not chosen.
      final off = installed.withLoadoutOff({'review'});
      final without = await reportChoosing({overlaysMeta: off.defaultOverlays});
      expect(env(without), isNull, reason: without.summary);
      final note = without.items.where((item) => item.key == 'overlay' && item.layer == 'overlay:review').single;
      expect(note.state, ItemState.off);
      expect(note.how, 'not chosen for this session');

      // The session that was chosen with it is unchanged by the default being off now: asking again as it did gives it.
      expect(env(await reportChoosing({overlaysMeta: const ['review']}))?.state, ItemState.applied);
    } finally {
      await host?.stop();
      // The library holds its home until it has let go.
      for (var i = 0; i < 10; i++) {
        try {
          temp.deleteSync(recursive: true);
          break;
        } on FileSystemException {
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
    }
  }, skip: problem);
}
