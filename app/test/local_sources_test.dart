// The host's own sources (Claude, Codex, ...): how the host's lines say what they are, how they become sources of the app,
// what the picker shows for them, and the harness their agent names itself as.
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/source_options.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _peer = 'sha256:0000000000000000000000000000000000000000000000000000000000000000';

HostListening _line(String json) => HostListening.parse(json)!;

void main() {
  test('a local source has no address, and says what it is missing', () {
    final claude = _line('{"name":"Claude","kind":"agent","local":true,"missing":["claude"],"harness":"claude","title":"Claude","peer":"$_peer"}');
    expect(claude.local, isTrue);
    expect(claude.url, isEmpty);
    expect(claude.missing, ['claude']);
    expect(claude.harness, 'claude');
    final network = _line('{"name":"x","kind":"agent","url":"ws://127.0.0.1:1","peer":"$_peer"}');
    expect(network.local, isFalse);
    expect(network.missing, isEmpty);
    expect(HostListening.parse('{"name":"x","kind":"agent","peer":"$_peer"}'), isNull, reason: 'a network entry needs its address');
  });

  test('the host\'s lines become hosted sources, the local ones marked', () {
    final sources = hostedSources([
      _line('{"name":"Claude","kind":"agent","local":true,"missing":[],"harness":"claude","peer":"$_peer"}'),
      _line('{"name":"Grok","kind":"agent","local":true,"missing":[],"peer":"$_peer"}'),
      _line('{"name":"mine","kind":"agent","url":"ws://127.0.0.1:1","peer":"$_peer"}'),
    ]);
    expect([for (final source in sources) (source.label, source.local)], [('Claude', true), ('Grok', true), ('mine', false)]);
    expect(sourceKey(sources.first), 'Claude');
  });

  test('a local source looks like its provider, and Claude and Codex offer their modes before they start', () {
    const claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
    const grok = AgentSource.hosted('Grok', local: true);
    const other = AgentSource.hosted('mine');
    expect(sourceProvider(claude), 'claude');
    expect(sourceProvider(grok), 'grok');
    expect(sourceProvider(other), isNull);
    expect(knownModes(claude), isNotEmpty);
    expect(knownModes(grok), isEmpty);
    expect(knownModes(other), isEmpty);
    expect(Icons.dns_outlined, providerIcon(null));
  });

  test('the translators the host runs are Claude Code and Codex, for the ledger', () {
    final names = HarnessNames();
    expect(names.label(reportedName: 'coqui-acp-claude', entry: 'Claude'), 'Claude Code');
    expect(names.label(reportedName: 'coqui-acp-codex', entry: 'Codex'), 'Codex');
  });
}
