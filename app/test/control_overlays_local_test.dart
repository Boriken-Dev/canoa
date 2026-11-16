// A desktop's overlay manager asks the host for the git work while the host is up, and does it on this computer's own disk while it
// is not; what only the disk answers (what is installed, where, the skills) is always the disk's. A page has no disk and asks the host.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/served/control_overlays.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostLink {
  final asked = <String>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    asked.add(request['method']! as String);
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': {'commit': 'c' * 40}}));
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
}

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

  test('a page has no disk: the host fetches, and it holds no skills', () async {
    final host = _Host();
    final manager = ControlOverlayManager(HostControl.dialing(() async => host));
    await manager.fetchCatalog('main', _source);
    expect(host.asked, ['catalog/fetch']);
    expect(manager.directoryOf('x'), 'overlays/x');
    expect(() => manager.removeSkill('s'), throwsA(isA<OverlayException>()));
  });
}
