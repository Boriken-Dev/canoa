// Registries, runtimes and the catalogs' cache are the host's work, asked over its control channel, so a served page and the
// desktop app read them in one place; while a desktop's host is down it does the same work itself.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/loadout/mcp_registry.dart';
import 'package:canoa/store/contents_storage.dart';
import 'package:canoa/store/control_data.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostLink {
  _Host(this.answer);

  /// The result for a method, or an error when it gives a string starting `error:`.
  final Object? Function(String method, Map<String, Object?> params) answer;
  final asked = <(String, Map<String, Object?>)>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    asked.add((method, params));
    final result = answer(method, params);
    final reply = result is String && result.startsWith('error:')
        ? {'error': {'code': -32002, 'message': result.substring(6)}}
        : {'result': result};
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

HostControl _over(_Host host) => HostControl.dialing(() async => host);

void main() {
  test('a registry page is read by the host from the address, and its words are the refusal', () async {
    final host = _Host((method, params) => params['url'].toString().contains('bad') ? 'error:The registry answered 404' : {'body': '{"servers":[]}'});
    final fetch = controlRegistryFetch(_over(host));
    expect(await fetch(Uri.parse('https://registry.example/v0.1/servers?limit=30')), '{"servers":[]}');
    expect(host.asked.single.$1, 'registry/fetch');
    expect(host.asked.single.$2['url'], 'https://registry.example/v0.1/servers?limit=30');
    await expectLater(fetch(Uri.parse('https://registry.example/bad/v0.1/servers')),
        throwsA(isA<RegistryRefusal>().having((e) => e.message, 'message', 'The registry answered 404')));
  });

  test('while the host is down the registry is read by the direct reader, and never when there is none', () async {
    final host = _Host((_, _) => {'body': 'host'});
    var up = false;
    final control = _over(host);
    final fetch = controlRegistryFetch(control, direct: (uri) async => 'direct', hostUp: () => up);
    expect(await fetch(Uri.parse('https://r.example/v0.1/servers')), 'direct');
    expect(host.asked, isEmpty);
    up = true;
    expect(await fetch(Uri.parse('https://r.example/v0.1/servers')), 'host');
    // A page has no direct reader: the host is the only way, up or not.
    final page = controlRegistryFetch(control, hostUp: () => false);
    expect(await page(Uri.parse('https://r.example/v0.1/servers')), 'host');
  });

  test('a runtime is found where the host says, and is missing when it says nothing or cannot answer', () async {
    final host = _Host((method, params) => {'found': {'npx': '/usr/bin/npx', 'uvx': null}});
    final find = controlFindProgram(_over(host));
    expect(await find('npx'), '/usr/bin/npx');
    expect(await find('uvx'), isNull);
    expect(host.asked.first.$1, 'host/which');
    expect(host.asked.first.$2['names'], ['npx']);
    final broken = controlFindProgram(_over(_Host((_, _) => 'error:no')));
    expect(await broken('npx'), isNull);
  });

  test('the document kept for a catalog is the host\'s, and a host without one gives none', () async {
    final kept = <String, String>{};
    final host = _Host((method, params) {
      if (method == 'store/contents_put') {
        kept['${params['label']}'] = '${params['document']}';
        return <String, Object?>{};
      }
      return {'document': kept['${params['label']}']};
    });
    final storage = ControlContentsStorage(_over(host));
    expect(await storage.read('main'), isNull);
    await storage.write('main', '{"version":1}');
    expect(await storage.read('main'), '{"version":1}');
    expect(await ControlContentsStorage(_over(_Host((_, _) => 'error:no'))).read('main'), isNull);
  });

  test('a desktop whose host is down keeps the documents itself', () async {
    final direct = MemoryContentsStorage();
    final host = _Host((_, _) => <String, Object?>{'document': null});
    var up = false;
    final storage = ControlContentsStorage(_over(host), direct: direct, hostUp: () => up);
    await storage.write('main', '{}');
    expect(direct.kept['main'], '{}');
    expect(host.asked, isEmpty);
    up = true;
    expect(await storage.read('main'), isNull);
  });
}
