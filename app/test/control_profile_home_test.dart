// A served page shows the layers, the project and the trust a desktop shows: the host reads the files and keeps the record, and the
// app reads what a layer says. The host here is a stand-in that answers `profile/*` and `trust/*` from a small disk of its own.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/profile/project_trust.dart';
import 'package:canoa/served/control_overlays.dart';
import 'package:canoa/served/control_profile_home.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostLink {
  _Host();

  /// The overlay folders by name, and the project profiles by working directory.
  final overlayFiles = <String, Map<String, String>>{
    '/ov/mine': {'overlay.cfg': '[harness.claude-code]\nmodel = "from-the-host"\n'},
  };
  final profiles = <String, String>{'/real/work': '[profile]\nmodel = "project-model"\n'};
  final links = {'/work': '/real/work'};
  final trusted = <Map<String, Object?>>[];
  final asked = <String>[];
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  Object? _answer(String method, Map<String, Object?> params) {
    switch (method) {
      case 'skill/list':
        return {'skills': <String, Object?>{}};
      case 'profile/canonical':
        final path = params['path']! as String;
        final real = links[path] ?? path;
        return {'directory': profiles.containsKey(real) || real.startsWith('/ov/') ? real : null};
      case 'profile/tree':
        final files = overlayFiles[params['directory']];
        if (files == null) return 'error:that is not the folder of an overlay on this host';
        return {
          'directory': params['directory'],
          'files': {for (final entry in files.entries) entry.key: base64Encode(utf8.encode(entry.value))},
        };
      case 'profile/project':
        final directory = links[params['directory']] ?? params['directory'];
        final text = profiles[directory];
        return {'directory': directory, 'profile': text == null ? null : base64Encode(utf8.encode(text))};
      case 'trust/list':
        return {'projects': trusted};
      case 'trust/set':
        final directory = links[params['directory']] ?? params['directory'];
        final text = profiles[directory]!;
        if (params['sha256'] != ProjectTrust.digest(utf8.encode(text))) return 'error:not the one that was shown';
        trusted
          ..removeWhere((each) => each['directory'] == directory)
          ..add({'directory': directory, 'sha256': params['sha256'], 'trusted': '2026-11-17T00:00:00Z'});
        return {'projects': trusted};
      case 'trust/revoke':
        trusted.removeWhere((each) => each['directory'] == (links[params['directory']] ?? params['directory']));
        return {'projects': trusted};
    }
    return 'error:unknown';
  }

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    final method = request['method']! as String;
    asked.add(method);
    final result = _answer(method, (request['params'] as Map?)?.cast<String, Object?>() ?? const {});
    final reply = result is String && result.startsWith('error:')
        ? {'error': {'code': -32002, 'message': result.substring(6)}}
        : {'result': result};
    _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], ...reply}));
  }

  @override
  Future<void> close() => _out.close();
}

void main() {
  late _Host host;
  late ControlProfileHome home;

  setUp(() {
    host = _Host();
    final control = HostControl.dialing(() async => host);
    addTearDown(host.close);
    home = ControlProfileHome(control, overlays: ControlOverlayManager(control), os: 'linux');
  });

  Future<ProfileLayers> layers() => profileLayers(
    overlays: {'mine': const DirOverlay('/ov/mine')},
    user: const Profile(),
    agents: const {},
    agent: 'Claude',
    home: home,
    workingDirectory: '/work',
  );

  test('shows an overlay the host read, a project the host found, and trust the host keeps', () async {
    var shown = await layers();
    expect(shown.notes.where((note) => note.layer.kind == LayerKind.overlay), isEmpty);
    final overlay = shown.layers.singleWhere((layer) => layer.id.kind == LayerKind.overlay);
    expect(overlay.profile.model, 'from-the-host');
    expect(shown.project?.directory, '/real/work', reason: 'the host resolved the link');
    expect(shown.project?.state, TrustState.untrusted);
    expect(shown.layers.any((layer) => layer.id.kind == LayerKind.project), isFalse);

    // Trusting hands the host what was shown; the host keeps it and the next reading uses it.
    await home.trust.trust(shown.project!.directory, shown.project!.profile!);
    expect(host.trusted.single['directory'], '/real/work');
    shown = await layers();
    expect(shown.project?.state, TrustState.trusted);
    expect(shown.layers.singleWhere((layer) => layer.id.kind == LayerKind.project).profile.model, 'project-model');

    // A profile that changed on the host since is not trusted until it is trusted again.
    host.profiles['/real/work'] = '[profile]\nmodel = "other"\n';
    expect((await layers()).project?.state, TrustState.changed);

    await home.trust.revoke('/real/work');
    expect(host.trusted, isEmpty);
    expect((await layers()).project?.state, TrustState.untrusted);
  });

  test('says why when the host will not read a folder, and what the host refused when it will not trust one', () async {
    host.overlayFiles.clear();
    final shown = await layers();
    expect(shown.layers.where((layer) => layer.id.kind == LayerKind.overlay), isEmpty);
    expect(shown.notes.map((note) => note.text).join('\n'), contains('not the folder of an overlay on this host'));

    host.profiles['/real/work'] = 'a';
    final project = (await layers()).project!;
    host.profiles['/real/work'] = 'b';
    await expectLater(
      home.trust.trust(project.directory, project.profile!),
      throwsA(isA<StateError>().having((error) => error.message, 'message', contains('not the one that was shown'))),
    );
    expect(host.trusted, isEmpty);
  });

  test('names a folder the host will not name as no folder', () async {
    expect(await home.canonicalDirectory('/nowhere'), isNull);
    expect(await home.readBytes('/nowhere/$projectProfileFile'), isNull);
    expect(await home.readBytes('/work/other.txt'), isNull, reason: 'only a project profile is read this way');
  });
}
