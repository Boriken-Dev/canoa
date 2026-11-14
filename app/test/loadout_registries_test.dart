// A registry the person adds beside the official one: an address that serves the MCP Registry's API (here a server on the loopback
// address, read by the app's own fetch), and a git repository holding a file in the API's list shape (here a bare repository). The
// controller is driven directly; the page's own cases are in loadout_test.dart.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_controller.dart';
import 'package:canoa/loadout/mcp_registry.dart';
import 'package:canoa/loadout/registry_fetch.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';
import 'mcp_registry_test.dart' show npmServer, page;

Map<String, Object?> teamServer() => {
  'name': 'dev.team/tracker',
  'description': 'The team tracker.',
  'version': '2.0.0',
  'remotes': [
    {'type': 'streamable-http', 'url': 'https://tracker.team.test/mcp'},
  ],
};

void main() {
  test('registries are a list of addresses in [loadout], in the order added, that the file keeps', () {
    final base = CanoaConfig.parse('');
    expect(base.loadoutRegistries, isEmpty);
    final two = base.withLoadoutRegistries(['https://registry.team.test', 'https://git.team.test/lists.git@main#servers.json']);
    final read = CanoaConfig.parse(two.encode());
    expect(read.loadoutRegistries, ['https://registry.team.test', 'https://git.team.test/lists.git@main#servers.json']);
    expect(CanoaConfig.parse(read.withLoadoutRegistries([]).encode()).loadoutRegistries, isEmpty);
    for (final text in [
      '[loadout]\nregistries = "https://x.test"\n',
      '[loadout]\nregistries = [1]\n',
      '[loadout]\nregistries = ["a b"]\n',
      '[loadout]\nregistries = ["https://x.test", "https://x.test"]\n',
    ]) {
      expect(() => CanoaConfig.parse(text), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('registries'))), reason: text);
    }
  });

  test('an address is a registry when it serves over https or the loopback address, or a git source of a file', () {
    for (final fine in ['https://registry.team.test', 'http://127.0.0.1:8080', 'https://git.team.test/lists.git@main#servers.json']) {
      expect(registryProblemOf(fine), isNull, reason: fine);
    }
    for (final bad in ['http://registry.team.test', 'https://u:p@registry.team.test', 'ftp://x.test', 'https://git.team.test/lists.git@main', 'https://u:p@git.team.test/l.git@main#s.json']) {
      expect(registryProblemOf(bad), isNotNull, reason: bad);
    }
  });

  test('a registry added by address is read over HTTP from its own list endpoint, and what is installed from it names it', () async {
    final seen = <Uri>[];
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) async {
      seen.add(request.uri);
      request.response.headers.contentType = ContentType.json;
      request.response.write(page([teamServer()]));
      await request.response.close();
    });
    final saved = <CanoaConfig>[];
    final controller = LoadoutController(
      config: CanoaConfig.parse(''),
      home: null,
      save: (changed) async {
        saved.add(changed);
        return changed;
      },
      agents: const ['Claude'],
      fetch: fetchRegistry,
    );
    try {
      final address = 'http://127.0.0.1:${server.port}';
      expect(controller.registries, [officialRegistry]);
      expect(await controller.addRegistry(address), isNull);
      expect(saved.last.loadoutRegistries, [address]);
      expect(controller.registries, [officialRegistry, address]);
      expect(controller.selectedRegistry, address);
      expect(controller.registryProblem, isNull);
      expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker']);
      expect(seen.single.path, '/v0.1/servers');

      final entry = controller.registryEntries.single;
      final option = optionsOf(entry.server).single;
      await controller.installMcp(entry, option, 'tracker');
      final origin = saved.last.loadoutMcp['tracker']!;
      expect(origin.registry, address, reason: 'installed from the registry it was listed by');
      expect(origin.server, 'dev.team/tracker');

      expect(await controller.addRegistry(address), 'That registry is already here');
      expect(await controller.addRegistry('http://registry.team.test'), isNotNull);
      await controller.removeRegistry(address);
      expect(saved.last.loadoutRegistries, isEmpty);
      expect(controller.selectedRegistry, officialRegistry);
    } finally {
      controller.dispose();
      await server.close(force: true);
    }
  });

  test('a git repository holding a list file is a registry: read at its ref, with no network but the repository', () async {
    final temp = await Directory.systemTemp.createTemp('canoa-registry-git');
    try {
      final root = temp.path.replaceAll(r'\', '/');
      final repo = Repo(root, 'lists')
        ..write('servers.json', page([teamServer(), npmServer()]))
        ..commit('the team list');
      final config = Directory('$root/config')..createSync();
      final saved = <CanoaConfig>[];
      final controller = LoadoutController(
        config: CanoaConfig.parse(''),
        home: IoProfileHome(config.path),
        save: (changed) async {
          saved.add(changed);
          return changed;
        },
        agents: const ['Claude'],
        // The official registry is not reachable here: nothing may read it.
        fetch: (uri) async => throw StateError('no HTTP registry is read: $uri'),
      );
      try {
        final spec = '${repo.location}@main#servers.json';
        expect(await controller.addRegistry(spec), isNull);
        expect(controller.registryProblem, isNull, reason: 'the list file was read');
        expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker', 'io.github.ana/files']);
        expect(controller.registryNext, isNull);

        // A search narrows what the file lists.
        await controller.loadRegistry(search: 'tracker');
        expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker']);

        final entry = controller.registryEntries.single;
        await controller.installMcp(entry, optionsOf(entry.server).single, 'tracker');
        expect(saved.last.loadoutMcp['tracker']!.registry, spec);

        // The file moves on: the next read is of the new commit.
        repo
          ..write('servers.json', page([npmServer()]))
          ..commit('the list changed');
        await controller.loadRegistry();
        expect(controller.registryEntries.map((entry) => entry.name), ['io.github.ana/files']);

        // A file the repository does not have is a line, not an exception.
        expect(await controller.addRegistry('${repo.location}@main#missing.json'), isNull);
        expect(controller.registryProblem, contains('missing.json'));
        expect(controller.registryEntries, isEmpty);

        // A file that is not a list is refused as any registry's answer is.
        repo
          ..write('servers.json', jsonEncode({'not': 'a list'}))
          ..commit('not a list');
        await controller.selectRegistry(spec);
        expect(controller.registryProblem, contains('not a list'));
      } finally {
        controller.dispose();
      }
    } finally {
      for (var i = 0; i < 10; i++) {
        try {
          temp.deleteSync(recursive: true);
          break;
        } on FileSystemException {
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
    }
  }, skip: hasGit ? null : 'git is not on PATH');
}
