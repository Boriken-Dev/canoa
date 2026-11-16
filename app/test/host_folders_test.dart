// Folders are the host's to answer, asked over its control channel, so a served page and the desktop app share one path: whether
// a folder is there and in a repository, an @'s names inside a session's folder, and a chat's scratch folder.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/chat/mentions.dart' show SessionMentions;
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/host_folders.dart';
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
        ? {'error': {'code': -32003, 'message': result.substring(6)}}
        : {'result': result};
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

Object? _info(String method, Map<String, Object?> params) => {
  'folders': [
    for (final path in params['paths']! as List<Object?>)
      {
        'path': path,
        'exists': !path.toString().contains('gone'),
        'folder': true,
        'repository': path.toString().contains('repo'),
        'root': path.toString().contains('repo') ? path : null,
      },
  ],
};

Future<void> _later() => Future<void>.delayed(const Duration(milliseconds: 20));

void main() {
  _mentionsOf();

  test('a recent folder is taken to be there, and a repository to be none, until the host says otherwise, and a widget is told', () async {
    final host = _Host(_info);
    final folders = HostFolders(HostControl.dialing(() async => host));
    addTearDown(folders.dispose);
    var told = 0;
    folders.addListener(() => told++);
    expect(folders.exists('/work/gone'), isTrue, reason: 'a guess first');
    expect(folders.inRepository('/work/repo'), isFalse);
    expect(folders.exists('/work/here'), isTrue);
    await _later();
    expect(folders.exists('/work/gone'), isFalse);
    expect(folders.inRepository('/work/repo'), isTrue);
    expect(folders.inRepository('/work/here'), isFalse);
    expect(told, greaterThan(0));
    expect(host.asked.where((call) => call.$1 == 'fs/info').length, 1, reason: 'one batched question, not one for each folder');
    expect(host.asked.single.$2['paths'], unorderedEquals(['/work/gone', '/work/repo', '/work/here']));
    // Asked again once, not every build.
    expect(folders.exists('/work/gone'), isFalse);
    await _later();
    expect(host.asked.length, 1);
  });

  test('an @ is offered the names the host lists, and a name is resolved by the host or not at all', () async {
    final host = _Host((method, params) => switch (method) {
      'fs/mentions' => {'entries': ['a.txt', 'src/']},
      'fs/resolve' => params['name'] == 'a.txt' ? {'path': '/w/a.txt', 'folder': false, 'size': 7} : 'error:not inside',
      _ => null,
    });
    final folders = HostFolders(HostControl.dialing(() async => host));
    addTearDown(folders.dispose);
    expect(await folders.mentions('/w'), ['a.txt', 'src/']);
    expect(host.asked.last.$2['path'], '/w');
    final found = await folders.resolve('/w', 'a.txt');
    expect(found, (path: '/w/a.txt', folder: false, size: 7));
    expect(await folders.resolve('/w', '../x'), isNull);
    final refused = HostFolders(HostControl.dialing(() async => _Host((_, _) => 'error:not a folder')));
    addTearDown(refused.dispose);
    expect(await refused.mentions('/nope'), isEmpty);
  });

  test('a chat\'s folder is made by the host from the session, the source and the local time, and taken back by the same words', () async {
    final host = _Host((method, params) => switch (method) {
      'fs/scratch' => {'path': '/docs/Canoa/scratch/s1', 'made': true},
      'fs/scratch_remove' => {'path': '/docs/Canoa/scratch/s1', 'removed': true},
      'fs/scratch_preview' => {'path': '/docs/example'},
      _ => null,
    });
    final folders = HostFolders(HostControl.dialing(() async => host));
    addTearDown(folders.dispose);
    final request = (session: 's1', source: 'Claude', handler: 'claude', created: DateTime(2026, 11, 17, 9, 5, 3));
    final made = await folders.makeScratch(request);
    expect(made, (path: '/docs/Canoa/scratch/s1', made: true));
    expect(host.asked.last.$2, {'session': 's1', 'source': 'Claude', 'handler': 'claude', 'now': '2026-11-17T09:05:03'});
    await folders.removeScratch(request);
    expect(host.asked.last.$1, 'fs/scratch_remove');
    expect(host.asked.last.$2['session'], 's1');
    expect(await folders.preview(r'${documents}/example/${session}'), '/docs/example');
    expect(host.asked.last.$2['template'], r'${documents}/example/${session}');
    final refusing = HostFolders(HostControl.dialing(() async => _Host((_, _) => 'error:no template')));
    addTearDown(refusing.dispose);
    await expectLater(refusing.makeScratch(request), throwsA(anything));
    expect(await refusing.preview('x'), isNull);
  });
}

void _mentionsOf() {
  test('an @ is offered for a session on a computer this app reads folders of, and only names the host resolved are offered and picked', () async {
    final store = SessionStore();
    final session = store.createDraft(cwd: '/w');
    session.place = SessionPlace.host;
    expect(SessionMentions.offeredFor(session), isFalse, reason: 'a host session is not this computer\'s');
    expect(SessionMentions.offeredFor(session, readable: (_) => true), isTrue);
    expect(SessionMentions.offeredFor(session, readable: (_) => false), isFalse);
    final mentions = SessionMentions(
      store: store,
      session: session,
      list: (_) async => ['a.txt', 'gone.txt', 'src/'],
      resolve: (folder, relative) async => relative == 'gone.txt' ? null : (path: '/w/$relative', folder: relative.endsWith('/'), size: 9),
    );
    final offered = await mentions.search('');
    expect(offered.map((each) => each.label), ['a.txt', 'src/'], reason: 'a name the host will not resolve is not offered');
    expect(mentions.pick(offered.first), isNull);
    expect(session.attachments.single.path, '/w/a.txt');
  });
}
