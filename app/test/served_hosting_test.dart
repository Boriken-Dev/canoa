// A page whose key holds `hosting` does what the desktop's Hosting page does, through the host's control channel: it lists, pairs and
// removes keys, shows a PIN that ends with the page's link, and reads, checks and writes `[host]` in the host's own words. A key
// without the grant gets neither the table nor a write of it.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/hosting_panel.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/served/host_config.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/served/served_app.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final _host = 'sha256:${'a' * 64}';
final _waiting = 'sha256:${'1' * 64}';
final _paired = 'sha256:${'2' * 64}';
final _joiner = 'sha256:${'3' * 64}';

/// What the host holds and was asked, shared by every link a page makes to it.
final class _Hosting {
  _Hosting({this.grants = const ['hosting']});

  final List<String> grants;
  final keys = <Map<String, Object?>>[
    {'id': _waiting, 'state': 'waiting', 'source': '127.0.0.1:50000'},
    {'id': _paired, 'state': 'accepted'},
  ];
  final asked = <(String, Map<String, Object?>)>[];
  Map<String, Object?>? table = {
    'agents': {
      'echo': {'program': 'echo', 'listen': '127.0.0.1:7811'},
    },
  };

  /// Which table is held, named as the host names it by a digest.
  var version = 0;

  /// Another writer changes the table after a page read it.
  void changedElsewhere(String agent) {
    table = {
      'agents': {
        ...?(table?['agents'] as Map?)?.cast<String, Object?>(),
        agent: {'program': 'echo', 'listen': '127.0.0.1:0'},
      },
    };
    version++;
  }

  /// The host's words for a table it will not take, or null when it takes it.
  String? wrong;

  /// What the pairing waits on: completed with a key's id when a program has used the PIN.
  Completer<String>? pairing;
  var closed = 0;

  List<String> methods(String name) => [for (final (method, _) in asked) if (method == name) method];
}

final class _Link implements HostLink {
  _Link(this._state);

  final _Hosting _state;
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  void _reply(Object? id, {Object? result, int? code, String? message}) {
    if (_out.isClosed) return;
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': id, if (code == null) 'result': result else 'error': {'code': code, 'message': message}}));
  }

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    final params = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    final id = request['id'];
    _state.asked.add((method, params));
    final needs = switch (method) {
      'host/keys' || 'host/accept' || 'host/revoke' || 'host/pair' => true,
      _ => params['section'] == 'host',
    };
    if (needs && !_state.grants.contains('hosting')) return _reply(id, code: -32001, message: 'this key has no hosting grant');
    switch (method) {
      case 'host/keys':
        return _reply(id, result: {'keys': _state.keys});
      case 'host/accept':
        final key = _state.keys.where((key) => key['id'] == params['id']).firstOrNull;
        if (key == null) return _reply(id, code: -32002, message: 'no key with that id is waiting: connect with it once, then accept it');
        key['state'] = 'accepted';
        return _reply(id, result: {'accepted': key['id']});
      case 'host/revoke':
        final before = _state.keys.length;
        _state.keys.removeWhere((key) => key['id'] == params['id']);
        return _reply(id, result: {'removed': _state.keys.length != before});
      case 'config/get':
        return _reply(id, result: {'section': params['section'], 'value': params['section'] == 'host' ? _state.table : null, 'restart': params['section'] == 'host', if (params['section'] == 'host') 'digest': 'v${_state.version}'});
      case 'config/check':
        return _state.wrong == null ? _reply(id, result: {'section': 'host', 'valid': true}) : _reply(id, code: -32002, message: _state.wrong);
      case 'config/set':
        if (_state.wrong != null) return _reply(id, code: -32002, message: _state.wrong);
        if (params['digest'] != null && params['digest'] != 'v${_state.version}') {
          return _reply(id, code: -32004, message: "the host's own settings changed since this page read them");
        }
        _state.table = (params['value'] as Map?)?.cast<String, Object?>();
        _state.version++;
        return _reply(id, result: {'section': 'host', 'restart': true, 'digest': 'v${_state.version}'});
      case 'host/pair':
        final done = _state.pairing = Completer<String>();
        _out.add(jsonEncode({
          'jsonrpc': '2.0',
          'method': 'host/progress',
          'params': {'id': id, 'pin': '482913', 'peer': _host, 'method': 'noise', if (params['native'] != true) 'url': 'ws://127.0.0.1:9100' else 'native': true},
        }));
        unawaited(done.future.then((who) => _reply(id, result: {'paired': who}), onError: (Object error) => _reply(id, code: -32002, message: '$error')));
        return;
    }
    _reply(id, code: -32601, message: 'this host has no such method');
  }

  @override
  Future<void> close() async {
    _state.closed++;
    final pairing = _state.pairing;
    // The host ends a pairing when the link that asked for it closes.
    if (pairing != null && !pairing.isCompleted) pairing.completeError('pairing was cancelled');
    await _out.close();
  }
}

ServedHost _served() => ServedHost(
  peer: _host,
  method: PeerMethod.noise,
  entries: const [],
  control: const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control'),
);

HostControl _over(_Hosting state) => HostControl.dialing(() async => _Link(state));

void main() {
  test('keys are listed with the paired first, and paired and removed by the host', () async {
    final state = _Hosting();
    final supervisor = ServedSupervisor(_served(), _over(state));
    expect(supervisor.managesKeys, isTrue);
    final keys = await supervisor.keys();
    expect([for (final key in keys) (key.id, key.accepted, key.from)], [(_paired, true, null), (_waiting, false, '127.0.0.1:50000')]);

    await supervisor.accept(_waiting);
    expect((await supervisor.keys()).every((key) => key.accepted), isTrue);
    await supervisor.revoke(_waiting);
    expect([for (final key in await supervisor.keys()) key.id], [_paired]);
    expect(state.asked.map((each) => each.$1), containsAll(['host/accept', 'host/revoke']));
    expect(state.asked.firstWhere((each) => each.$1 == 'host/accept').$2, {'id': _waiting});
  });

  test('a refusal is the host\'s own words, and a key without the grant is turned away', () async {
    final supervisor = ServedSupervisor(_served(), _over(_Hosting()));
    await expectLater(
      supervisor.accept('sha256:${'9' * 64}'),
      throwsA(isA<HostCommandFailed>().having((error) => error.message, 'message', contains('no key with that id is waiting'))),
    );
    final without = ServedSupervisor(_served(), _over(_Hosting(grants: const ['ledger.read'])));
    await expectLater(without.keys(), throwsA(isA<HostCommandFailed>().having((error) => error.message, 'message', contains('hosting'))));
    // A page with no control channel manages nothing.
    expect(ServedSupervisor(_served()).managesKeys, isFalse);
  });

  test('a PIN is shown from the host, the key that uses it is named, and cancelling closes the link the host holds it by', () async {
    final state = _Hosting();
    final supervisor = ServedSupervisor(_served(), _over(state));
    final shown = await supervisor.pair();
    expect((shown.pin, shown.hostId, shown.url, shown.method, shown.native), ('482913', _host, 'ws://127.0.0.1:9100', 'noise', false));
    expect(state.asked.single.$1, 'host/pair');
    expect(state.asked.single.$2, isEmpty);
    state.pairing!.complete(_joiner);
    expect(await shown.paired, _joiner);
    await Future<void>.delayed(Duration.zero);
    expect(state.closed, 1, reason: 'the link of its own ends with the pairing');

    final second = await supervisor.pair();
    final ended = expectLater(second.paired, throwsA(isA<HostCommandFailed>()));
    second.cancel();
    await ended;
    expect(state.closed, 2);
  });

  test('the extension\'s pairing is asked for natively, and a host that cannot pair says why', () async {
    final state = _Hosting();
    final supervisor = ServedSupervisor(_served(), _over(state));
    final shown = await supervisor.pairBrowser();
    expect((shown.native, shown.url), (true, ''));
    expect(state.asked.single.$2, {'native': true});
    shown.cancel();
    await expectLater(shown.paired, throwsA(isA<HostCommandFailed>()));

    final without = ServedSupervisor(_served(), _over(_Hosting(grants: const [])));
    await expectLater(without.pair(), throwsA(isA<HostCommandFailed>().having((error) => error.message, 'message', contains('hosting'))));
  });

  group('[host] on the host', () {
    Future<(HostConfigStore, CanoaConfig)> load(_Hosting state) async {
      final store = HostConfigStore(HostFiles(_over(state)));
      return (store, await store.load());
    }

    test('is read with the page\'s settings by a key that holds hosting, and written back only when it changed', () async {
      final state = _Hosting();
      final (store, config) = await load(state);
      expect(store.includesHost, isTrue);
      expect(config.hostEntries.map((entry) => entry.name), ['echo']);
      expect(await store.save(config), isFalse, reason: 'nothing changed, so nothing is sent');
      expect(state.methods('config/set'), isEmpty);

      final changed = config.withHostAgent(name: 'review', program: 'agy', args: const ['acp'], listen: '127.0.0.1:0', workingRoots: const []);
      expect(await store.checkHost(changed), isNull);
      expect(await store.save(changed), isTrue, reason: 'the host reads [host] when it starts');
      final written = state.asked.where((each) => each.$1 == 'config/set').single;
      expect(written.$2['section'], 'host');
      expect(((written.$2['value'] as Map)['agents'] as Map).keys, containsAll(['echo', 'review']));
      expect(state.methods('config/check'), hasLength(1));
    });

    test('a write made after another writer changed the table is refused, and the page reads the table again', () async {
      final state = _Hosting();
      final (store, config) = await load(state);
      state.changedElsewhere('added-meanwhile');
      final changed = config.withHostAgent(name: 'review', program: 'agy', args: const [], listen: '127.0.0.1:0', workingRoots: const []);
      await expectLater(
        store.save(changed),
        throwsA(isA<HostConfigChanged>().having((error) => error.config.hostEntries.map((entry) => entry.name), 'names', contains('added-meanwhile'))),
      );
      expect(((state.table!['agents']) as Map).keys, isNot(contains('review')), reason: 'the host kept what the other writer wrote');

      // Read again, the same change is taken, and a second one is not refused by the first's own write.
      final fresh = await store.load();
      final again = fresh.withHostAgent(name: 'review', program: 'agy', args: const [], listen: '127.0.0.1:0', workingRoots: const []);
      expect(await store.save(again), isTrue);
      expect(((state.table!['agents']) as Map).keys, containsAll(['echo', 'added-meanwhile', 'review']));
      final more = again.withHostAgent(name: 'third', program: 'agy', args: const [], listen: '127.0.0.1:0', workingRoots: const []);
      expect(await store.save(more), isTrue);
    });

    test('the app that runs the host writes the tables it changed, its own among them, in the file\'s format', () async {
      final state = _Hosting();
      final config = CanoaConfig.parse('[desktop]\nstart_hidden = false\n');
      final store = HostConfigStore.ofDesktop(HostFiles(_over(state)), config);
      expect(store.loaded, isTrue);
      expect(await store.save(config), isFalse, reason: 'nothing changed, so nothing is sent');
      expect(state.methods('config/set'), isEmpty);

      final changed = config.copyWith(startHidden: true, closeToTray: true);
      await store.save(changed);
      final written = [for (final (method, params) in state.asked) if (method == 'config/set') params];
      expect([for (final each in written) each['section']], ['desktop'], reason: 'only the table that changed');
      expect((written.single['value'] as Map)['start_hidden'], true);
      expect(await store.save(changed), isFalse, reason: 'and it is not sent again');

      // A file written by the app itself meanwhile is what the next change is measured from.
      store.adopt(config);
      await store.save(changed);
      expect(state.methods('config/set'), hasLength(2));
    });

    test('a table the host will not take comes back in its words and is not written', () async {
      final state = _Hosting();
      final (store, config) = await load(state);
      state.wrong = 'that would leave canoa.cfg refused: host.agents.review: program or harness is needed';
      final changed = config.withHostAgent(name: 'review', program: 'agy', args: const [], listen: '127.0.0.1:0', workingRoots: const []);
      expect(await store.checkHost(changed), 'that would leave canoa.cfg refused: host.agents.review: program or harness is needed');
      expect(state.methods('config/set'), isEmpty);
    });

    test('a key without the grant reads and writes no host table, and is not failed by it', () async {
      final state = _Hosting(grants: const ['config.read']);
      final (store, config) = await load(state);
      expect(store.includesHost, isFalse);
      expect(config.hostEntries, isEmpty);
      expect(await store.checkHost(config), contains('may not change'));
      expect(await store.save(config), isFalse);
      expect(state.methods('config/set'), isEmpty);
    });
  });

  testWidgets('the Hosting panel on a page lists keys, pairs one after the person compares its id, and shows a PIN', (tester) async {
    final state = _Hosting();
    final supervisor = ServedSupervisor(_served(), _over(state));
    await tester.binding.setSurfaceSize(const Size(900, 1400));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.runAsync(() async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: HostingPanel(supervisor: supervisor, config: const CanoaConfig(), onSave: (_) async => null, freePort: () async => 0),
            ),
          ),
        ),
      );
      await Future<void>.delayed(const Duration(milliseconds: 100));
    });
    await tester.pump();
    expect(find.text('Serving this page.'), findsOneWidget);
    expect(find.text(_waiting), findsOneWidget);

    await tester.tap(find.text('Pair…'));
    await tester.pumpAndSettle();
    expect(find.text('Pair this key?'), findsOneWidget);
    expect(state.methods('host/accept'), isEmpty, reason: 'nothing is accepted by opening the dialog');
    await tester.tap(find.text('Pair'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pump(const Duration(seconds: 1));
    expect(state.methods('host/accept'), hasLength(1));

    await tester.tap(find.text('Show a PIN'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pump();
    expect(find.text('482913'), findsOneWidget);
    expect(find.text('Dial ws://127.0.0.1:9100'), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));
    await tester.tap(find.text('Cancel'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pump();
    expect(state.closed, greaterThanOrEqualTo(1));
  });
}
