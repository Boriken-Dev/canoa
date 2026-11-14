// What a page does with the host that served it, without a host: its entries as sources, the probe that says whether the page's key
// is accepted on the control channel, the folder dialog over `fs/list`, and an attachment kept on the host.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/served_host.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/chat/composer.dart';
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/served/folder_dialog.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/served/host_config.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show PickedFile;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final _peer = 'sha256:${'a' * 64}';

ServedHost _host({bool control = true}) => ServedHost(
  peer: _peer,
  method: PeerMethod.noise,
  entries: const [
    ServedEntry(name: 'scripted', kind: 'agent', url: 'ws://127.0.0.1:1/scripted', title: 'Scripted'),
    ServedEntry(name: 'tools', kind: 'tool', url: 'ws://127.0.0.1:1/tools'),
    ServedEntry(name: 'claude', kind: 'agent', url: 'ws://127.0.0.1:1/claude', harness: 'claude'),
  ],
  control: control ? const ServedEntry(name: '@control', kind: 'control', url: 'ws://127.0.0.1:1/control') : null,
);

/// A control channel whose host answers each request with [answer].
final class _Link implements HostLink {
  _Link(this.answer);

  final Object? Function(String method, Map<String, Object?> params) answer;
  final _out = StreamController<String>();
  final asked = <String>[];

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    asked.add(request['method']! as String);
    try {
      final result = answer(request['method']! as String, request['params']! as Map<String, Object?>);
      _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
    } on Object catch (error) {
      _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32001, 'message': '$error'}}));
    }
  }

  @override
  Future<void> close() => _out.close();
}

void main() {
  group('a host that served the page', () {
    test('lists its agent entries as sources, by the host\'s key and method, and never a tool or its control channel', () {
      final supervisor = ServedSupervisor(_host());
      expect(supervisor.phase, HostPhase.running);
      expect(supervisor.peerId, _peer);
      expect(supervisor.peerMethod, 'noise');
      expect([for (final entry in supervisor.listening) entry.name], ['scripted', 'claude']);
      expect(supervisor.listening.every((entry) => entry.peer == _peer && entry.method == 'noise' && !entry.local), isTrue);
      expect(supervisor.listening.first.title, 'Scripted');
      expect(supervisor.listening.last.harness, 'claude');
    });

    test('starts and stops nothing', () async {
      final supervisor = ServedSupervisor(_host());
      await supervisor.apply(configPath: '/anywhere');
      await supervisor.restart();
      await supervisor.stop();
      expect(supervisor.phase, HostPhase.running);
      expect(supervisor.listening, hasLength(2));
    });

    test('a probe finds a host that listens for no control channel, and says the sessions stay in the browser', () async {
      final app = ServedApp.over(_host(control: false), HostControl.dialing(() async => throw StateError('never dialled')));
      await app.probe();
      expect(app.reachable, isFalse);
      expect(app.problem, contains('no control channel'));
    });

    test('a probe finds a key the host turned away, and says what to do', () async {
      final app = ServedApp.over(_host(), HostControl.dialing(() async => throw StateError('the host turned this key away')));
      await app.probe();
      expect(app.reachable, isFalse);
      expect(app.problem, contains('[host.control]'));
      // What the failure said is for the log: it may come from an endpoint the page has not proven.
      expect(app.problem, isNot(contains('turned this key away')));
      expect(app.detail, contains('turned this key away'));
    });

    test('a probe asks the host something, since a key it grants nothing is turned away only after the proof', () async {
      final link = _Link((method, params) => method == 'ledger/list' ? {'records': <Object?>[]} : throw StateError('not asked'));
      final app = ServedApp.over(_host(), HostControl.dialing(() async => link));
      await app.probe();
      expect(app.reachable, isTrue);
      expect(link.asked, ['ledger/list']);
      await app.close();
    });

    test('a key without the ledger grant is not reachable, however the proof went', () async {
      final link = _Link((method, params) => throw StateError('denied'));
      final app = ServedApp.over(_host(), HostControl.dialing(() async => link));
      await app.probe();
      expect(app.reachable, isFalse);
      await app.close();
    });
  });

  group('canoa.cfg on the host', () {
    /// A host whose canoa.cfg holds [tables], by section; the requests it was sent are kept.
    (_Link, HostFiles) hostWith(Map<String, Map<String, Object?>> tables) {
      final link = _Link((method, params) => switch (method) {
        'config/get' => {'section': params['section'], 'value': tables[params['section']], 'restart': false},
        'config/set' => {'section': params['section'], 'restart': params['section'] == 'permissions'},
        _ => throw StateError('not asked: $method'),
      });
      return (link, HostFiles(HostControl.dialing(() async => link)));
    }

    test('a table this page cannot read fails the whole read, and nothing is then written, so nothing is removed', () async {
      final (link, files) = hostWith({
        'permissions': {'a_key_this_build_does_not_know': true},
        'overlays': {'docs': 'https://example.com/docs.git'},
      });
      final store = HostConfigStore(files);
      await expectLater(store.load(), throwsA(anything));
      expect(store.loaded, isFalse);
      await expectLater(store.save(const CanoaConfig()), throwsA(isA<StateError>()));
      expect(link.asked.where((method) => method == 'config/set'), isEmpty);
    });

    test('a page that read it whole sends only the table the person changed, not tables the host spells another way', () async {
      final (link, files) = hostWith({
        'appearance': {'density': 'verbose', 'brightness': 'dark'},
        'sessions': {'workspace': r'${documents}/a/${session}'},
      });
      final store = HostConfigStore(files);
      final config = await store.load();
      expect(store.loaded, isTrue);
      // Saving what was read changes nothing on the host.
      expect(await store.save(config), isFalse);
      expect(link.asked.where((method) => method == 'config/set'), isEmpty);
      final changed = CanoaConfig.parse(
        jsonEncode({'appearance': {'density': 'verbose', 'brightness': 'light'}, 'sessions': {'workspace': r'${documents}/a/${session}'}}),
        name: 'canoa.json',
      );
      await store.save(changed);
      expect(link.asked.where((method) => method == 'config/set'), hasLength(1));
    });

    test('a table the host had is removed only when the person emptied it, and one it never had is never sent as removed', () async {
      final removed = <String>[];
      final link = _Link((method, params) {
        if (method == 'config/get') return {'section': params['section'], 'value': params['section'] == 'sessions' ? {'workspace': r'${documents}/x/${session}'} : null};
        removed.add('${params['section']}=${params['value']}');
        return {'section': params['section'], 'restart': false};
      });
      final store = HostConfigStore(HostFiles(HostControl.dialing(() async => link)));
      await store.load();
      await store.save(CanoaConfig.parse('{}', name: 'canoa.json'));
      expect(removed, ['sessions=null']);
    });

    test('the app reads none of it when a table is unreadable: no config, a fixed sentence, and nothing to write against', () async {
      final link = _Link((method, params) => {
        'section': params['section'],
        'value': params['section'] == 'permissions' ? {'a_key_this_build_does_not_know': true} : null,
      });
      final app = ServedApp.over(_host(), HostControl.dialing(() async => link));
      expect(await app.loadConfig(), isNull);
      expect(app.configLoaded, isFalse);
      expect(app.configProblem, isNotNull);
      expect(app.configProblem, isNot(contains('a_key_this_build_does_not_know')));
    });

    test('the host\'s operating system comes from what it lists, and an overlay is read for it', () {
      final served = ServedHost.parse(jsonEncode({'peer': _peer, 'method': 'noise', 'entries': <Object?>[], 'os': 'windows'}))!;
      expect(served.os, 'windows');
      expect(ServedApp.over(served, HostControl.dialing(() async => throw StateError('x'))).home.os, 'windows');
      expect(ServedHost.parse(jsonEncode({'peer': _peer, 'method': 'noise', 'entries': <Object?>[]}))!.os, isNull);
    });
  });

  group('the folder dialog', () {
    HostFolderListing listing(String? path, String? parent, List<(String, String)> folders) => HostFolderListing(
      path: path,
      parent: parent,
      entries: [for (final (name, at) in folders) HostFolderEntry(name: name, path: at, isFolder: true)],
      more: false,
    );

    Future<void> open(WidgetTester tester, ListHostFolder list, {String? start}) async {
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () => unawaited(pickHostFolder(context, list, start: start)),
            child: const Text('open'),
          ),
        ),
      ));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
    }

    testWidgets('starts at the places the host offers, goes into a folder and up again, and chooses one', (tester) async {
      final asked = <String?>[];
      Future<HostFolderListing> list(String? path) async {
        asked.add(path);
        return switch (path) {
          null => listing(null, null, [('home', '/home/ana')]),
          '/home/ana' => listing('/home/ana', null, [('work', '/home/ana/work'), ('notes', '/home/ana/notes')]),
          '/home/ana/work' => listing('/home/ana/work', '/home/ana', const []),
          _ => throw StateError('not listed'),
        };
      }

      String? chosen;
      var done = false;
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () => pickHostFolder(context, list).then((path) {
              chosen = path;
              done = true;
            }),
            child: const Text('open'),
          ),
        ),
      ));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Where to start'), findsOneWidget);
      // Nothing is chosen at the starting places.
      expect(tester.widget<FilledButton>(find.byKey(const Key('host-folder-choose'))).onPressed, isNull);
      await tester.tap(find.byKey(const ValueKey('host-folder /home/ana')));
      await tester.pumpAndSettle();
      expect(find.text('/home/ana'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('host-folder /home/ana/work')));
      await tester.pumpAndSettle();
      expect(find.text('No folders here.'), findsOneWidget);
      await tester.tap(find.byKey(const Key('host-folder-up')));
      await tester.pumpAndSettle();
      expect(find.text('/home/ana'), findsOneWidget);
      expect(asked, [null, '/home/ana', '/home/ana/work', '/home/ana']);
      await tester.tap(find.byKey(const Key('host-folder-choose')));
      await tester.pumpAndSettle();
      expect(done, isTrue);
      expect(chosen, '/home/ana');
    });

    testWidgets('falls back to the starting places when the folder it was given is not one the host will list', (tester) async {
      final asked = <String?>[];
      Future<HostFolderListing> list(String? path) async {
        asked.add(path);
        if (path != null) throw StateError('nothing there');
        return listing(null, null, [('home', '/home/ana')]);
      }

      await open(tester, list, start: '/gone');
      expect(asked, ['/gone', null]);
      expect(find.byKey(const ValueKey('host-folder /home/ana')), findsOneWidget);
      expect(find.byKey(const Key('host-folder-problem')), findsNothing);
    });

    testWidgets('says why a folder cannot be listed, in the host\'s words, and keeps the one it showed', (tester) async {
      Future<HostFolderListing> list(String? path) async {
        if (path == '/home/ana/secret') throw StateError('the host refused: not found (-32003)');
        return path == null ? listing(null, null, [('secret', '/home/ana/secret')]) : listing(path, null, const []);
      }

      await open(tester, list);
      await tester.tap(find.byKey(const ValueKey('host-folder /home/ana/secret')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('host-folder-problem')), findsOneWidget);
      expect(find.textContaining('not found'), findsOneWidget);
      expect(find.byKey(const ValueKey('host-folder /home/ana/secret')), findsOneWidget);
    });
  });

  group('an attachment on a page with a host', () {
    Future<(SessionStore, ChatSession)> draft() async {
      final store = SessionStore();
      final session = store.createDraft();
      return (store, session);
    }

    PickedFile picked(String name, List<int> bytes, {String? mime}) =>
        (name: name, path: '', mimeType: mime, read: () async => Uint8List.fromList(bytes));

    test('a file that is not a small image is kept on the host and attached by the path it answers', () async {
      final (store, session) = await draft();
      final kept = <(String, int)>[];
      final kit = ComposerKit(uploadFile: (name, bytes) async {
        kept.add((name, bytes.length));
        return (path: '/home/ana/.local/share/canoa/uploads/ab12-$name', size: bytes.length);
      });
      final problems = await SessionAttachments(store, session, kit).addFiles([
        picked('data.bin', List.filled(2048, 7)),
        picked('notes.txt', utf8.encode('hello')),
      ]);
      expect(problems, isEmpty);
      expect(kept, [('data.bin', 2048), ('notes.txt', 5)]);
      expect([for (final attachment in session.attachments) attachment.name], ['data.bin', 'notes.txt']);
      expect(session.attachments.first.kind, AttachmentKind.file);
      expect(session.attachments.first.path, '/home/ana/.local/share/canoa/uploads/ab12-data.bin');
    });

    test('a session on another peer gets no upload, since its agent is not on the host that served the page', () async {
      final (store, session) = await draft();
      var uploads = 0;
      final kit = ComposerKit(
        hostFiles: (each) => false,
        uploadFile: (name, bytes) async {
          uploads++;
          return (path: '/x', size: bytes.length);
        },
      );
      final problems = await SessionAttachments(store, session, kit).addFiles([picked('data.bin', [0xff, 0xfe, 0xfd])]);
      expect(uploads, 0);
      expect(problems.single, contains('not text'));
    });

    test('a small image goes in the message, as on a desktop, and is not kept', () async {
      final (store, session) = await draft();
      var uploads = 0;
      final kit = ComposerKit(uploadFile: (name, bytes) async {
        uploads++;
        return (path: '/x', size: bytes.length);
      });
      final problems = await SessionAttachments(store, session, kit).addFiles([picked('dot.png', List.filled(64, 1), mime: 'image/png')]);
      expect(problems, isEmpty);
      expect(uploads, 0);
      expect(session.attachments.single.kind, AttachmentKind.image);
    });

    test('a refusal of the host is the file\'s problem, in its words, and nothing is attached', () async {
      final (store, session) = await draft();
      final kit = ComposerKit(uploadFile: (name, bytes) async => throw StateError('the host refused: too large (-32002)'));
      final problems = await SessionAttachments(store, session, kit).addFiles([picked('big.bin', List.filled(16, 1))]);
      expect(problems.single, allOf(startsWith('big.bin:'), contains('too large')));
      expect(session.attachments, isEmpty);
    });

    test('without a host, a file that is not text is still refused', () async {
      final (store, session) = await draft();
      final problems = await SessionAttachments(store, session, const ComposerKit()).addFiles([picked('data.bin', [0xff, 0xfe, 0xfd])]);
      expect(problems.single, contains('not text'));
    });
  });
}
