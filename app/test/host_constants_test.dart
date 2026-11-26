// The few facts the app keeps beside the host, since it shows them with no host (a page nothing served, a desktop whose host is
// stopped): each is the host's own, read here from the host's source, so a change in one without the other fails. What a host can
// say is read from it instead: the official MCP registry and the default catalog come from `store/catalogs`, and which kinds of
// client the host knows is the host's to refuse.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:flutter_test/flutter_test.dart';

String _host(String file) =>
    File('../crates/canoa-host/src/$file').readAsStringSync();

/// The text of the Rust constant [name] in [source]: the string it is set to, which may be on the next line.
String _constant(String source, String name) {
  final found = RegExp('const $name: &str =\\s*"([^"]+)";').firstMatch(source);
  expect(found, isNotNull, reason: '$name is not found in the host');
  return found!.group(1)!;
}

void main() {
  test('the catalog offered first is the host\'s', () {
    final index = _host('store/index.rs');
    expect(defaultCatalogLabel, _constant(index, 'DEFAULT_CATALOG_LABEL'));
    expect(defaultCatalogSpec, _constant(index, 'DEFAULT_CATALOG_SPEC'));
  });

  test('the skills directory a search asks is the host\'s', () {
    expect(
      CanoaConfig.defaultSkillsSearch,
      _constant(_host('control/skill_search.rs'), 'DEFAULT_BASE'),
    );
  });

  test('every key of [host.kb] the app edits is one the host reads', () {
    final raw = _host('config/raw.rs');
    Set<String> fieldsOf(String struct) {
      final body = RegExp('struct $struct \\{([^}]+)\\}').firstMatch(raw);
      expect(body, isNotNull, reason: struct);
      return {
        for (final field in RegExp(
          r'^\s*(?:pub(?:\([a-z]+\))?\s+)?(?:r#)?(\w+):',
          multiLine: true,
        ).allMatches(body!.group(1)!))
          field.group(1)!,
      };
    }

        expect(fieldsOf('RawKb').containsAll(['embedder', 'ranker']), isTrue);
    for (final role in ['RawKbEmbedder', 'RawKbRanker']) {
      expect(
        fieldsOf(role).containsAll(['use', 'model']),
        isTrue,
        reason: role,
      );
    }
    for (final key in CanoaConfig.hostKbKeys.where((key) => key != 'server')) {
      expect(
        ['embedder', 'ranker'].contains(key.split('_').first),
        isTrue,
        reason: key,
      );
      expect(
        ['use', 'model'].contains(key.split('_').last),
        isTrue,
        reason: key,
      );
    }
    // The server is a `[sdk.NAME]` table of the `server` provider with an address and the operations it does.
    expect(raw, contains('"provider" | "address" | "key_env"'));
    expect(
      _host('../../canoa/src/sdk/openai_shape.rs'),
      contains('pub const SERVER_ID: &str = "server";'),
    );
  });

  test('every key of [assist_harness.NAME] and [assist_pool] and [assist.OPERATION] the app edits is one the host reads', () {
    final raw = _host('config/raw.rs');
    Set<String> fieldsOf(String struct) {
      final body = RegExp('struct $struct \\{([^}]+)\\}').firstMatch(raw);
      expect(body, isNotNull, reason: struct);
      return {
        for (final field in RegExp(r'^\s*(?:pub(?:\([a-z]+\))?\s+)?(?:r#)?(\w+):', multiLine: true).allMatches(body!.group(1)!)) field.group(1)!,
      };
    }

    expect(fieldsOf('RawAssistHarness').containsAll(CanoaConfig.assistHarnessKeys), isTrue);
    expect(fieldsOf('RawAssistPool').containsAll(['idle_seconds', 'max_sessions']), isTrue);
    expect(fieldsOf('RawAssist').containsAll(['use', 'fallback']), isTrue);
  });
}

