// The app's reading of the host's `_canoa/mcp/list`: every page the host has, and a server the agent starts as pending.
import 'dart:convert';

import 'package:canoa/acp/mcp_list.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the app reads every page the host answers, with no page limit of its own', () async {
    late McpListWire wire;
    var asked = 0;
    wire = McpListWire((message) async {
      final request = jsonDecode(message) as Map<String, Object?>;
      final params = request['params'] as Map<String, Object?>;
      final at = int.parse((params['cursor'] as String?) ?? '0');
      asked++;
      final last = at == 49;
      Future<void>.microtask(
        () => wire.takes(
          jsonEncode({
            'jsonrpc': '2.0',
            'id': request['id'],
            'result': {
              'tools': [
                {'server': 's', 'name': 't$at'},
              ],
              'resources': <Object?>[],
              'servers': [
                {'name': 's', 'status': 'ok', 'tools': 50, 'resources': 0},
              ],
              'nextCursor': ?(last ? null : '${at + 1}'),
            },
          }),
        ),
      );
    });
    final inventory = await wire.list('session');
    expect(asked, 50);
    expect(inventory!.tools.length, 50);
    expect(inventory.tools.last.name, 't49');
  });

  test('a server the agent starts is pending, not failed', () {
    final status = McpServerStatus.parse({'name': 'theirs', 'status': 'pending', 'note': 'the agent starts this server', 'tools': 0, 'resources': 0})!;
    expect(status.pending, isTrue);
    expect(status.ok, isFalse);
    expect(status.error, isNull);
    final failed = McpServerStatus.parse({'name': 'x', 'status': 'error', 'error': 'no'})!;
    expect(failed.pending, isFalse);
  });
}
