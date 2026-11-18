// The Skills and MCP pages read every source and registry through the real host's `store/catalog`: a skills source written as a
// folder, and a registry written as a git source of a list file, both kept by the `read` provider in canoa.cfg. Needs git,
// CANOA_TEST_HOST (a built canoa) and CANOA_TEST_LIBRARY (a built canoa_ffi).
@Timeout(Duration(minutes: 5))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa/loadout/loadout_controller.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/served/control_overlays.dart';
import 'package:canoa/store/catalog_types.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart' show embeddedHostMissing;
import 'fixtures/store_fixtures.dart';

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

String _servers(List<String> names) => jsonEncode({
  'servers': [
    for (final name in names)
      {
        'server': {
          'name': name,
          'description': 'The $name server.',
          'version': '1.0.0',
          'remotes': [
            {'type': 'streamable-http', 'url': 'https://example.test/$name'},
          ],
        },
      },
  ],
  'metadata': {'count': names.length},
});

void main() {
  final problem = !hasGit ? 'git is not on PATH' : embeddedHostMissing();

  test('a skills source and an MCP registry kept by the read provider are listed through store/catalog, as the page reads them', () async {
    final temp = await Directory.systemTemp.createTemp('canoa-catalog-host');
    HostSupervisor? host;
    try {
      final root = temp.path.replaceAll(r'\', '/');
      final skills = Repo(root, 'skills')
        ..write('skills/pdf/SKILL.md', '---\nname: pdf\ndescription: Reads PDFs.\n---\nBody.\n')
        ..write('skills/docx/SKILL.md', '---\nname: docx\ndescription: Makes Word documents.\n---\nBody.\n');
      final skillsCommit = skills.commit('skills');
      final lists = Repo(root, 'lists')..write('servers.json', _servers(['dev.team/tracker', 'io.github.ana/files']));
      lists.commit('servers');
      final registry = '${lists.location}@main#servers.json';
      final folder = skills.work.path.replaceAll(r'\', '/');
      final config = Directory('$root/config')..createSync();
      final base = CanoaConfig.parse('[store.skills.catalogs]\nanthropics = false\nteam = "$folder"\n'
          '[store.mcps.catalogs]\nlists = "$registry"\n');
      await File('${config.path}/canoa.cfg').writeAsString(base.encode());
      host = HostSupervisor(LibraryHostTool(
        CanoaNative.open(path: File(Platform.environment['CANOA_TEST_LIBRARY']!).absolute.path),
        home: '${config.path}/home',
        program: File(Platform.environment['CANOA_TEST_HOST']!).absolute.path,
      ));
      await host.apply(configPath: '${config.path}/canoa.cfg');
      await _until(() => host!.phase == HostPhase.idle, 'the host to start');
      final control = HostControl(host);

      // The host's own answers, which the page reads as they are.
      final read = controlCatalogRead(control)!;
      final scanned = await read('skills', 'team');
      expect(scanned['provider'], 'read');
      expect(scanned['commit'], skillsCommit);
      expect([for (final skill in scanned['skills']! as List) (skill as Map)['name']]..sort(), ['docx', 'pdf']);
      expect(((scanned['skills']! as List).first as Map)['source'], folder);
      final page = await read('mcps', 'lists');
      expect([for (final each in page['servers']! as List) ((each as Map)['server'] as Map)['name']], ['dev.team/tracker', 'io.github.ana/files']);
      expect(await read('mcps', 'lists', search: 'tracker').then((answer) => (answer['servers']! as List).length), 1);
      await expectLater(read('skills', 'ghost'), throwsA(isA<CatalogReadFailed>()));

      // The page's controller over it: nothing is read until a source is chosen, then the host's answers are what it lists.
      final saved = <CanoaConfig>[];
      late final LoadoutController controller;
      controller = LoadoutController(
        config: base,
        home: IoProfileHome(config.path, overlays: ControlOverlayManager(control)),
        save: (changed) async {
          saved.add(changed);
          return changed;
        },
        agents: const ['Claude'],
        readCatalog: read,
      );
      try {
        expect(controller.skillScans, isEmpty);
        await controller.scanSkillSource('team');
        expect(controller.skillScanProblems, isEmpty);
        final scan = controller.skillScans['team']!;
        expect(scan.commit, skillsCommit);
        expect(scan.skills.map((skill) => '${skill.name}:${skill.path}').toList()..sort(), ['docx:skills/docx', 'pdf:skills/pdf']);

        expect(controller.registryLabelOf(registry), 'lists');
        await controller.selectRegistry(registry);
        expect(controller.registryProblem, isNull);
        expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker', 'io.github.ana/files']);
        await controller.loadRegistry(search: 'files');
        expect(controller.registryEntries.map((entry) => entry.name), ['io.github.ana/files']);
      } finally {
        controller.dispose();
      }
    } finally {
      await host?.stop();
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
