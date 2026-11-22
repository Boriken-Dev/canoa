// A registry the person adds beside the official one: an address that serves the MCP Registry's API (here a server on the loopback
// address, read by the app's own fetch), and a git repository holding a file in the API's list shape (here a bare repository). The
// controller is driven directly; the page's own cases are in loadout_test.dart.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/catalog_specs.dart' show CatalogSpec;
import 'package:canoa/loadout/loadout_controller.dart';
import 'package:canoa/loadout/mcp_registry.dart';
import 'package:canoa/store/catalog_types.dart' show CatalogRow, CatalogType, CatalogWords;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
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

/// What the host's `store/catalog` fetches for a registry's list, against a server of the test's own.
Future<String> _read(Uri uri) async {
  final client = HttpClient();
  try {
    return await utf8.decodeStream(await (await client.getUrl(uri)).close());
  } finally {
    client.close();
  }
}

void main() {
  test('the official registry is the one the host says, and none until it has said', () async {
    final controller = LoadoutController(
      config: CanoaConfig.parse(''),
      home: null,
      save: (changed) async => changed,
      catalogTypes: () async => [
        CatalogType(
          id: 'mcps',
          title: 'MCP servers',
          words: CatalogWords.generic,
          rows: [CatalogRow(label: 'official', spec: CatalogSpec.read('https://registry.host-says.test'), fixed: true)],
        ),
      ],
    );
    addTearDown(controller.dispose);
    expect(controller.registries, isEmpty, reason: 'the app holds no address of its own');
    await controller.loadTypes();
    expect(controller.registries, ['https://registry.host-says.test']);
    expect(controller.selectedRegistry, 'https://registry.host-says.test');
  });

  test('registries are the catalogs of the MCP servers type, in the order added, that the file keeps', () {
    final base = CanoaConfig.parse('');
    expect(base.storeRegistries, isEmpty);
    final two = base.withStoreRegistries(['https://registry.team.test', 'https://git.team.test/lists.git@main#servers.json']);
    final read = CanoaConfig.parse(two.encode());
    expect(read.storeRegistries, ['https://registry.team.test', 'https://git.team.test/lists.git@main#servers.json']);
    expect(CanoaConfig.parse(read.withStoreRegistries([]).encode()).storeRegistries, isEmpty);
    for (final text in [
      '[store.mcps.catalogs]\none = 1\n',
      '[store.mcps.catalogs]\none = "a b"\n',
      '[store.mcps.catalogs]\none = "https://x.test"\ntwo = "https://x.test"\n',
      '[store]\nmcps = "https://x.test"\n',
    ]) {
      expect(() => CanoaConfig.parse(text), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('store'))), reason: text);
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
    late final LoadoutController controller;
    controller = LoadoutController(
      config: CanoaConfig.parse(''),
      home: null,
      save: (changed) async {
        saved.add(changed);
        return changed;
      },
      agents: const ['Claude'],
      catalogTypes: standInCatalogTypes,
      readCatalog: standInCatalogRead(MemoryOverlays(), () => controller.config, registry: _read),
    );
    await controller.loadTypes();
    try {
      final address = 'http://127.0.0.1:${server.port}';
      expect(controller.registries, [officialRegistry]);
      expect(await controller.addRegistry(address), isNull);
      expect(saved.last.storeRegistries, [address]);
      expect(controller.registries, [officialRegistry, address]);
      expect(controller.selectedRegistry, address);
      expect(controller.registryProblem, isNull);
      expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker']);
      expect(seen.single.path, '/v0.1/servers');

      final entry = controller.registryEntries.single;
      final option = optionsOf(entry.server).single;
      await controller.installMcp(entry, option, 'tracker');
      final origin = saved.last.storeMcp['tracker']!;
      expect(origin.registry, address, reason: 'installed from the registry it was listed by');
      expect(origin.server, 'dev.team/tracker');

      expect(await controller.addRegistry(address), 'That registry is already here');
      expect(await controller.addRegistry('http://registry.team.test'), isNotNull);
      await controller.removeRegistry(address);
      expect(saved.last.storeRegistries, isEmpty);
      expect(controller.selectedRegistry, officialRegistry);
    } finally {
      controller.dispose();
      await server.close(force: true);
    }
  });

  test('a git repository holding a list file is a registry: read at its ref, with no network but the repository', () async {
    // The repository is read through the host's catalog calls, which this stand-in answers from memory.
    const location = 'https://git.example/lists.git';
    final overlays = MemoryOverlays()..put(location, {'servers.json': page([teamServer(), npmServer()])});
    final saved = <CanoaConfig>[];
    late final LoadoutController controller;
    controller = LoadoutController(
      config: CanoaConfig.parse(''),
      home: MemoryHome(overlays),
      save: (changed) async {
        saved.add(changed);
        return changed;
      },
      agents: const ['Claude'],
      catalogTypes: standInCatalogTypes,
      // The official registry is not reachable here: nothing may read it.
      readCatalog: standInCatalogRead(overlays, () => controller.config,
          registry: (uri) async => throw StateError('no HTTP registry is read: $uri')),
    );
    try {
      final spec = '$location@main#servers.json';
      expect(await controller.addRegistry(spec), isNull);
      expect(controller.registryProblem, isNull, reason: 'the list file was read');
      expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker', 'io.github.ana/files']);
      expect(controller.registryNext, isNull);

      // A search narrows what the file lists.
      await controller.loadRegistry(search: 'tracker');
      expect(controller.registryEntries.map((entry) => entry.name), ['dev.team/tracker']);

      final entry = controller.registryEntries.single;
      await controller.installMcp(entry, optionsOf(entry.server).single, 'tracker');
      expect(saved.last.storeMcp['tracker']!.registry, spec);

      // The file moves on: the next read is of the new commit.
      overlays.put(location, {'servers.json': page([npmServer()])});
      await controller.loadRegistry();
      expect(controller.registryEntries.map((entry) => entry.name), ['io.github.ana/files']);

      // A file the repository does not have is a line, not an exception.
      expect(await controller.addRegistry('$location@main#missing.json'), isNull);
      expect(controller.registryProblem, contains('missing.json'));
      expect(controller.registryEntries, isEmpty);

      // A file that is not a list is refused as any registry's answer is.
      overlays.put(location, {'servers.json': jsonEncode({'not': 'a list'})});
      await controller.selectRegistry(spec);
      expect(controller.registryProblem, contains('not a list'));

      // The official registry can also be removed: it is written as `official = false`.
      await controller.removeRegistry(officialRegistry);
      expect((saved.last.catalogTable('mcps') as Map?)?['official'], isFalse);
      expect(controller.registries, isNot(contains(officialRegistry)));
    } finally {
      controller.dispose();
    }
  });
}
