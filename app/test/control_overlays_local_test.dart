// A desktop's overlay manager asks the host for the git work while the host is up, and does it on this computer's own disk while it
// is not; what only the disk answers (what is installed, where) is always the disk's. A page has no disk and asks the host.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/skill_copies.dart';
import 'package:canoa/served/control_overlays.dart';
import 'package:coqui_aos/coqui_aos.dart' as aos;
import 'package:flutter_test/flutter_test.dart';

import 'package:canoa/profile/aos_support.dart' show treeOfFiles;

final class _Host implements HostLink {
  _Host([this.answer]);

  /// What a method answers, when it is not the commit of a catalog fetch.
  final Object? Function(String method, Map<String, Object?> params)? answer;
  final asked = <String>[];
  final params = <Map<String, Object?>>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    final given = (request['params'] as Map?)?.cast<String, Object?>() ?? const {};
    asked.add(method);
    params.add(given);
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': answer?.call(method, given) ?? {'commit': 'c' * 40}}));
  }

  @override
  Future<void> close() => _out.close();
}

final class _Disk extends Fake implements OverlayManager {
  final used = <String>[];

  @override
  InstalledOverlay? installed(String name) => null;

  @override
  String directoryOf(String name) => '/disk/$name';

  @override
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source) async {
    used.add('fetchCatalog');
    throw const OverlayException('disk');
  }

  @override
  Future<void> removeCatalog(String label) async => used.add('removeCatalog');

  @override
  Future<Map<String, SkillCopy>> skillCopies() async {
    used.add('skillCopies');
    return const {};
  }

  @override
  Future<void> removeSkill(String name) async => used.add('removeSkill');
}

const _skillText = '---\nname: review\ndescription: Does a thing.\n---\n# review\n';

final _source = OverlaySource.parseGit('https://git.example/c.git@main#catalog.toml');

void main() {
  test('with the host up the catalog is fetched by the host, and the paths are the disk\'s', () async {
    final host = _Host();
    final disk = _Disk();
    final manager = ControlOverlayManager(HostControl.dialing(() async => host), local: disk, hostUp: () => true);
    final tree = await manager.fetchCatalog('main', _source);
    expect(tree.commit, 'c' * 40);
    await manager.removeCatalog('main');
    expect(host.asked, ['catalog/fetch', 'catalog/remove']);
    expect(disk.used, isEmpty);
    expect(manager.directoryOf('x'), '/disk/x');
  });

  test('with the host down the disk does the work and the host is not asked', () async {
    final host = _Host();
    final disk = _Disk();
    final manager = ControlOverlayManager(HostControl.dialing(() async => host), local: disk, hostUp: () => false);
    await expectLater(manager.fetchCatalog('main', _source), throwsA(isA<OverlayException>().having((e) => e.message, 'message', 'disk')));
    await manager.removeCatalog('main');
    expect(disk.used, ['fetchCatalog', 'removeCatalog']);
    expect(host.asked, isEmpty);
  });

  test('a page has no disk: the host fetches', () async {
    final host = _Host();
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    await manager.fetchCatalog('main', _source);
    expect(host.asked, ['catalog/fetch']);
    expect(manager.directoryOf('x'), 'overlays/x');
  });

  test('a page reviews, keeps and removes skills through the host, and reads each digest\'s files once', () async {
    final files = {'SKILL.md': utf8.encode(_skillText), 'run.sh': utf8.encode('echo hi\n')};
    final digest = '${treeOfFiles(files).digest.digest}';
    final encoded = {for (final MapEntry(:key, :value) in files.entries) key: base64Encode(value)};
    final host = _Host((method, params) => switch (method) {
      'skill/list' => {
        'skills': {
          'review': {'name': 'review', 'source': 'git+https://git.example/t.git@main', 'digest': digest, 'overlays': ['team'], 'chosen': true},
        },
      },
      'skill/read' => {'files': encoded},
      'skill/fetch' => {'commit': 'c' * 40, 'files': encoded},
      _ => {'saved': true, 'removed': true},
    });
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));

    final copies = await manager.skillCopies();
    expect(copies.keys, ['review']);
    expect(copies['review']!.chosen, isTrue);
    expect(copies['review']!.files['run.sh'], utf8.encode('echo hi\n'));
    await manager.skillCopies();
    expect(host.asked, ['skill/list', 'skill/read', 'skill/list']);

    final fetched = await manager.fetchSkillFiles(aos.Source.parse('https://git.example/t.git@main#skills/review'));
    expect({for (final (path, bytes) in fetched) path: bytes.length}, {'SKILL.md': files['SKILL.md']!.length, 'run.sh': 8});
    expect(host.params.last, {'repo': 'https://git.example/t.git', 'ref': 'main', 'path': 'skills/review'});
    await expectLater(manager.fetchSkillFiles(aos.Source.parse('https://user:secret@git.example/t.git@main')), throwsA(isA<OverlayException>()));

    await manager.saveSkill(copies['review']!.copyWith(chosen: false));
    final saved = host.params.last;
    expect(host.asked.last, 'skill/save');
    expect(saved['name'], 'review');
    expect(saved['chosen'], false);
    expect(saved['digest'], digest);
    expect((saved['files']! as Map)['run.sh'], base64Encode(utf8.encode('echo hi\n')));

    await manager.removeSkill('review');
    expect(host.asked.last, 'skill/remove');
    expect(host.params.last, {'name': 'review'});
  });

  test('a desktop whose host is down keeps its skills on its own disk, and asks the host while it is up', () async {
    final host = _Host((method, params) => method == 'skill/list' ? {'skills': {}} : {'removed': true});
    final disk = _Disk();
    var up = false;
    final manager = ControlOverlayManager(HostControl.dialing(() async => host), local: disk, hostUp: () => up);
    await manager.skillCopies();
    await manager.removeSkill('review');
    expect(disk.used, ['skillCopies', 'removeSkill']);
    expect(host.asked, isEmpty);
    up = true;
    await manager.skillCopies();
    await manager.removeSkill('review');
    expect(host.asked, ['skill/list', 'skill/remove']);
    expect(disk.used, ['skillCopies', 'removeSkill']);
  });
}
