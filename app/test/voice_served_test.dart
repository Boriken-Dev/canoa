// Dictation on the page a host served: the host's speech providers are read when the page opens, so the first press of the
// microphone already has the service canoa.cfg names, and a page whose host has a local provider and no choice made dictates
// through it. The control channel is a stand-in that answers `speech/*`; no microphone is opened.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';

/// A host's control channel that grants [grants], lists [providers] and recognizes whatever audio arrives as [text].
final class _Control implements HostLink {
  _Control({required this.grants, this.providers = const [], this.offers = const []});

  final List<String> grants;
  final List<Map<String, Object?>> providers;
  final List<Map<String, Object?>> offers;
  final text = 'run the tests';
  final calls = <String>[];
  var pushed = 0;
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    calls.add(method);
    final Object? result = switch (method) {
      'host/grants' => {'grants': grants},
      'ledger/list' => {'records': <Object?>[]},
      'speech/providers' => grants.contains('voice') ? {'types': <Object?>[], 'providers': providers} : null,
      'speech/offers' => grants.contains('voice') ? {'offers': offers} : null,
      'speech/open' => {'session': 'sp-1', 'partials': false},
      'speech/push' => () {
        pushed++;
        return {'partial': null};
      }(),
      'speech/finish' => {'text': text, 'front_matter': <String, Object?>{}},
      'speech/cancel' => <String, Object?>{},
      _ => null,
    };
    _out.add(jsonEncode(result == null
        ? {'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32001, 'message': 'this key holds no grant for that'}}
        : {'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
  }

  @override
  Future<void> close() => _out.close();
}

/// A microphone that plays two chunks of silence and ends, as a stopped one does.
final class _Capture implements AudioCapture {
  int opened = 0;

  @override
  Stream<Uint8List> open({required int rate}) async* {
    opened++;
    yield Uint8List(3200);
    yield Uint8List(3200);
  }
}

const _local = {
  'name': 'whisper',
  'provider': 'speech_command',
  'title': 'Command',
  'description': 'Runs a program that reads audio and writes text',
  'settings': <String, Object?>{},
  'secrets_set': <Object?>[],
  'partials': false,
  'offer': 'whisper',
};

/// The same provider, written by hand in canoa.cfg: the host knows nothing of where its program sends the audio.
final _own = <String, Object?>{..._local, 'name': 'mine', 'offer': null};

const _whisper = {
  'id': 'whisper',
  'title': 'whisper.cpp',
  'summary': 'Speech to text on the host.',
  'where': 'The audio goes to your host and is turned into text there.',
  'local': true,
  'models': <Object?>[],
  'installed': true,
};

ServedHost _host() => ServedHost(
  peer: 'sha256:${'a' * 64}',
  method: PeerMethod.noise,
  entries: const [ServedEntry(name: 'Agent', kind: 'agent', url: 'ws://127.0.0.1:1/agent')],
  control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
);

Future<_Capture> _page(WidgetTester tester, _Control control, {VoiceSettings voice = const VoiceSettings()}) async {
  final capture = _Capture();
  final host = _host();
  final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
  await pumpApp(
    tester,
    store,
    config: CanoaConfig(agents: const [scripted], voice: voice),
    host: ServedSupervisor(host),
    served: ServedApp.over(host, HostControl.dialing(() async => control)),
    capture: capture,
  );
  await newSession(tester);
  // The page reads what its host offers once it is open.
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
  await frames(tester);
  return capture;
}

Future<void> _dictate(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('dictate')));
  await frames(tester);
  if (find.byKey(const Key('dictate-accept')).evaluate().isNotEmpty) {
    await tester.tap(find.byKey(const Key('dictate-accept')));
    await frames(tester);
  }
  // The capture is over at once, which ends the recording and asks the host for the text.
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
  await frames(tester);
}

void main() {
  testWidgets('a served page whose canoa.cfg names a host provider dictates through it on the first press', (tester) async {
    final control = _Control(grants: const ['ledger.read', 'voice'], providers: const [_local]);
    final capture = await _page(tester, control, voice: const VoiceSettings(backend: 'host:whisper'));
    expect(find.byKey(const Key('dictate')), findsOneWidget, reason: 'the page shows the microphone');
    await _dictate(tester);
    expect(find.byKey(const Key('dictate-error')), findsNothing);
    expect(capture.opened, 1);
    expect(control.pushed, greaterThan(0));
    expect(control.calls, containsAllInOrder(['speech/open', 'speech/push', 'speech/finish']));
    expect(find.text('run the tests'), findsOneWidget, reason: 'the text lands in the message');
    await finish(tester);
  });

  testWidgets('a served page with no choice made and a local provider on its host dictates through that provider', (tester) async {
    final control = _Control(grants: const ['ledger.read', 'voice'], providers: const [_local], offers: const [_whisper]);
    final capture = await _page(tester, control);
    await _dictate(tester);
    expect(find.byKey(const Key('dictate-error')), findsNothing);
    expect(capture.opened, 1);
    expect(find.text('run the tests'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a provider the person wrote by hand is never chosen for them', (tester) async {
    final control = _Control(grants: const ['ledger.read', 'voice'], providers: [_own], offers: const [_whisper]);
    final capture = await _page(tester, control);
    await tester.tap(find.byKey(const Key('dictate')));
    await frames(tester);
    expect(capture.opened, 0, reason: 'nothing was recorded');
    expect(find.byKey(const Key('dictate-error')), findsOneWidget);
    expect(control.calls, isNot(contains('speech/open')));
    await finish(tester);
  });

  testWidgets('a page whose host gives its key no voice grant says so instead of listing a service', (tester) async {
    final control = _Control(grants: const ['ledger.read']);
    await _page(tester, control);
    expect(control.calls, isNot(contains('speech/providers')), reason: 'a key with no voice grant is not asked');
    await tester.tap(find.byKey(const Key('dictate')));
    await frames(tester);
    final said = tester.widget<Text>(find.byKey(const Key('dictate-error'))).data!;
    expect(said, contains('voice'));
    await finish(tester);
  });
}
