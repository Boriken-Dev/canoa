// The overlay manager asks the host for all the git work, for a page and a desktop alike.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
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

const _skillText = '---\nname: review\ndescription: Does a thing.\n---\n# review\n';

final _source = OverlaySource.parseGit('https://git.example/c.git@main#catalog.toml');

void main() {
  test('the host fetches, and what is installed is what it last said', () async {
    final host = _Host();
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    await manager.fetchCatalog('main', _source);
    expect(host.asked, ['catalog/fetch']);
    expect(manager.directoryOf('x'), 'overlays/x');
  });

  test('the host says which installed overlays have an update, and a refresh asks it to look now', () async {
    final host = _Host((method, params) => switch (method) {
      'overlay/updates' => {
        'overlays': {
          'team': {'state': 'available', 'installed': 'a' * 40, 'latest': 'b' * 40, 'follows': 'branch'},
          'old': {'state': 'unreachable', 'installed': 'c' * 40, 'problem': 'no route'},
          'odd': 3,
        },
        'available': 1,
      },
      _ => {},
    });
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    final found = await manager.updates();
    expect(host.params.last, <String, Object?>{});
    expect(found.keys, ['team', 'old']);
    expect(found['team']!.available, isTrue);
    expect(found['team']!.latest, 'b' * 40);
    expect(found['old']!.available, isFalse);
    expect(found['old']!.problem, 'no route');
    await manager.updates(refresh: true);
    expect(host.asked, ['overlay/updates', 'overlay/updates']);
    expect(host.params.last, {'refresh': true});
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

  test('a source is scanned through the host, as the person\'s own only when it says so, and a fetch carries the same choice', () async {
    final host = _Host((method, params) => switch (method) {
      'skill/scan' => {
        'commit': 'd' * 40,
        'skills': [
          {'name': 'pdf', 'description': 'Reads PDFs.', 'path': 'skills/pdf'},
          {'name': 'odd', 'description': 'Odd.', 'path': 'skills/other', 'problem': 'the name has to be its folder\'s'},
          {'path': 'skills/nameless'},
        ],
        'skipped': 2,
        'truncated': false,
      },
      _ => {'commit': 'd' * 40, 'files': {'SKILL.md': base64Encode(utf8.encode(_skillText))}},
    });
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    final scan = await manager.scanSkills('https://github.com/anthropics/skills.git');
    expect(host.asked.last, 'skill/scan');
    expect(host.params.last, {'repo': 'https://github.com/anthropics/skills.git', 'ref': null, 'path': ''});
    expect(scan.commit, 'd' * 40);
    expect([for (final skill in scan.skills) (skill.name, skill.path, skill.problem != null)], [('pdf', 'skills/pdf', false), ('odd', 'skills/other', true)]);
    expect(scan.skipped, 2);

    await manager.scanSkills('https://git.example/o.git', ref: 'v1.0.0', path: 'skills', own: true);
    expect(host.params.last, {'repo': 'https://git.example/o.git', 'ref': 'v1.0.0', 'path': 'skills', 'own': true});
    await expectLater(manager.scanSkills('https://user:secret@git.example/t.git'), throwsA(isA<OverlayException>()));

    await manager.fetchSkillFiles(aos.Source.parse('https://git.example/t.git@main#skills/review'), own: true);
    expect(host.params.last, {'repo': 'https://git.example/t.git', 'ref': 'main', 'path': 'skills/review', 'own': true});
  });

  test('a search goes to the host as the words typed, and only a result naming a GitHub owner/repo is kept', () async {
    final host = _Host((method, params) => {
      'from': 'https://skills.sh',
      'skills': [
        {'id': 'anthropics/skills/pdf', 'source': 'anthropics/skills', 'skillId': 'pdf', 'name': 'pdf', 'installs': 231770},
        {'id': 'x', 'source': 'https://evil.example/o/r', 'skillId': 'pdf', 'name': 'pdf', 'installs': 9},
        {'id': 'y', 'source': '/etc', 'skillId': 'passwd', 'name': 'p', 'installs': 9},
        'text',
      ],
    });
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    final hits = await manager.searchSkills('pdf tools');
    expect(host.asked.last, 'skill/search');
    expect(host.params.last, {'query': 'pdf tools'});
    expect([for (final hit in hits) (hit.id, hit.installs)], [('anthropics/skills/pdf', 231770)]);
  });
}
