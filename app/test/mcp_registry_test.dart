// MCP servers from a registry: the list endpoint's address and answer, what installing each way to run a server would write,
// and what Canoa refuses and says why. The shapes are those of the MCP Registry's API and `server.json`.
import 'dart:convert';

import 'package:canoa/loadout/mcp_registry.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> npmServer({List<Object?> env = const [], List<Object?> args = const []}) => {
  'name': 'io.github.ana/files',
  'description': 'Reads files.',
  'version': '1.2.0',
  'repository': {'url': 'https://github.com/ana/files', 'source': 'github'},
  'packages': [
    {
      'registryType': 'npm',
      'identifier': '@ana/files-mcp',
      'version': '1.2.0',
      'transport': {'type': 'stdio'},
      'environmentVariables': env,
      'packageArguments': args,
    },
  ],
};

String page(List<Map<String, Object?>> servers, {String? next, bool latest = true}) => jsonEncode({
  'servers': [
    for (final server in servers)
      {
        'server': server,
        '_meta': {'io.modelcontextprotocol.registry/official': {'status': 'active', 'isLatest': latest}},
      },
  ],
  'metadata': {'count': servers.length, 'nextCursor': ?next},
});

void main() {
  group('the list address', () {
    test('is the registry\'s v0.1 servers endpoint with the limit, cursor and search', () {
      final uri = listUri('https://registry.example.com/', search: ' git ', cursor: 'a:1', limit: 10);
      expect(uri.toString(), 'https://registry.example.com/v0.1/servers?limit=10&cursor=a%3A1&search=git');
    });

    test('is https, or http for this computer only, and carries no user information', () {
      expect(() => listUri('http://registry.example.com'), throwsA(isA<RegistryRefusal>()));
      expect(() => listUri('https://user:pw@registry.example.com'), throwsA(isA<RegistryRefusal>()));
      expect(() => listUri('https://registry.example.com?x=1'), throwsA(isA<RegistryRefusal>()));
      expect(listUri('http://127.0.0.1:8080').host, '127.0.0.1');
    });
  });

  group('a listing', () {
    test('reads each server and the next cursor, and leaves out versions the registry says are not the latest', () {
      final read = parseRegistryPage(page([npmServer()], next: 'files:1.2.0'));
      expect(read.entries.single.name, 'io.github.ana/files');
      expect(read.entries.single.installName, 'files');
      expect(read.entries.single.repositoryUrl, 'https://github.com/ana/files');
      expect(read.next, 'files:1.2.0');
      expect(parseRegistryPage(page([npmServer()], latest: false)).entries, isEmpty);
    });

    test('is refused when it is not a list, and the digest is of the server.json whatever order its keys are in', () {
      expect(() => parseRegistryPage('{"servers": 3}'), throwsA(isA<RegistryRefusal>()));
      expect(() => parseRegistryPage('not json'), throwsA(isA<RegistryRefusal>()));
      final a = McpRegistryEntry({'name': 'x/y', 'version': '1', 'description': 'd'});
      final b = McpRegistryEntry({'description': 'd', 'version': '1', 'name': 'x/y'});
      expect(a.digest, b.digest);
      expect(a.digest, matches(RegExp(r'^sha256:[0-9a-f]{64}$')));
      expect(McpRegistryEntry({'name': 'x/y', 'version': '2'}).digest, isNot(a.digest));
    });
  });

  group('what installing writes', () {
    test('an npm package is run with npx -y, and a secret is read from the environment, never written', () {
      final option = optionsOf(npmServer(env: [
        {'name': 'FILES_TOKEN', 'isRequired': true, 'isSecret': true, 'description': 'A token'},
        {'name': 'FILES_MODE', 'default': 'read'},
      ], args: [
        {'type': 'named', 'name': '--root', 'value': '/srv'},
        {'type': 'positional', 'value': 'serve'},
      ])).single;
      expect(option.refusal, isNull);
      expect(option.runtime, 'npx');
      expect(option.definition, {
        'type': 'stdio',
        'command': 'npx',
        'args': ['-y', '@ana/files-mcp@1.2.0', '--root', '/srv', 'serve'],
        'env': {
          'FILES_TOKEN': {'from_env': 'FILES_TOKEN'},
          'FILES_MODE': {'value': 'read'},
        },
      });
      expect(option.needs.single.describe(), contains('read from your environment variable FILES_TOKEN'));
      expect(option.needs.single.describe(), contains('never writes to canoa.cfg'));
      expect(jsonEncode(option.definition), isNot(contains('secret value')));
    });

    test('a pypi package is run with uvx and a container with docker, which passes only the variables it names', () {
      final py = optionsOf({
        'name': 'a/b',
        'packages': [
          {'registryType': 'pypi', 'identifier': 'b-mcp', 'version': '0.3', 'transport': {'type': 'stdio'}},
        ],
      }).single;
      expect(py.definition!['command'], 'uvx');
      expect(py.definition!['args'], ['b-mcp==0.3']);
      final oci = optionsOf({
        'name': 'a/c',
        'packages': [
          {
            'registryType': 'oci',
            'identifier': 'ghcr.io/ana/c:1.0',
            'transport': {'type': 'stdio'},
            'environmentVariables': [
              {'name': 'C_KEY', 'isRequired': true, 'isSecret': true},
            ],
          },
        ],
      }).single;
      expect(oci.definition!['command'], 'docker');
      expect(oci.definition!['args'], ['run', '-i', '--rm', '-e', 'C_KEY', 'ghcr.io/ana/c:1.0']);
      expect(oci.runtime, 'docker');
    });

    test('a remote is connected to over its own address, with a header read from the environment', () {
      final option = optionsOf({
        'name': 'a/d',
        'remotes': [
          {
            'type': 'streamable-http',
            'url': 'https://mcp.example.com/{tenant}/mcp',
            'variables': {'tenant': {'default': 'acme'}},
            'headers': [
              {'name': 'X-Api-Key', 'isRequired': true, 'isSecret': true},
            ],
          },
        ],
      }).single;
      expect(option.refusal, isNull);
      expect(option.runtime, isNull);
      expect(option.definition, {
        'type': 'http',
        'url': 'https://mcp.example.com/acme/mcp',
        'headers': {'X-Api-Key': {'from_env': 'X_API_KEY'}},
      });
    });
  });

  group('what is refused, and why', () {
    String? why(Map<String, Object?> server) => optionsOf(server).single.refusal;

    test('a bundle, a package of a registry Canoa builds no command for, and a server that is not on its standard streams', () {
      expect(why({'packages': [{'registryType': 'mcpb', 'identifier': 'https://x.test/a.mcpb'}]}), contains('mcpb'));
      expect(why({'packages': [{'registryType': 'cargo', 'identifier': 'a-mcp', 'version': '1'}]}), contains('cargo'));
      expect(why({'packages': [{'registryType': 'npm', 'identifier': 'a', 'transport': {'type': 'streamable-http', 'url': 'http://localhost:1'}}]}),
          contains('standard streams'));
    });

    test('a remote over plain http to another computer, with user information, or with a placeholder it cannot fill', () {
      expect(why({'remotes': [{'type': 'streamable-http', 'url': 'http://mcp.example.com/mcp'}]}), contains('plain http'));
      expect(why({'remotes': [{'type': 'streamable-http', 'url': 'https://u:p@mcp.example.com/mcp'}]}), contains('user information'));
      expect(why({'remotes': [{'type': 'streamable-http', 'url': 'https://mcp.example.com/{tenant}'}]}), contains('{tenant}'));
      expect(why({'remotes': [{'type': 'websocket', 'url': 'wss://mcp.example.com'}]}), contains('websocket'));
    });

    test('an identifier or an argument Canoa would not put on a command line, or a value it cannot choose', () {
      expect(why({'packages': [{'registryType': 'npm', 'identifier': 'a; rm -rf /', 'version': '1'}]}), contains('characters'));
      expect(why(npmServer(args: [{'type': 'positional', 'isRequired': true, 'valueHint': 'a path'}])), contains('a path'));
    });

    test('a server with neither a package nor a remote has nothing to install', () {
      expect(optionsOf({'name': 'a/e'}), isEmpty);
    });
  });
}
