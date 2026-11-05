// The profile vectors canoa is held to as well (crates/canoa-host/tests/profile.rs): each merge's view or first
// refusal, and each overlay source.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/profile/aos.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:canoa/profile/profile.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final vectors = jsonDecode(File('../crates/canoa-host/tests/vectors/profile.json').readAsStringSync()) as Map;

  List<Layer> layers(Map<dynamic, dynamic> vector) => [
    for (final (index, layer as Map) in (vector['layers'] as List).indexed)
      () {
        final kind = LayerKind.named(layer['kind'] as String)!;
        try {
          return Layer(LayerId(kind, (layer['name'] as String?) ?? ''), Profile.parse(layer['toml'] as String, kind));
        } on ProfileException catch (error) {
          throw (index, error);
        }
      }(),
  ];

  test('the vectors are all there', () {
    expect((vectors['merges'] as List).length, greaterThan(40));
    expect((vectors['sources'] as List).length, greaterThan(15));
  });

  for (final vector in (vectors['merges'] as List).cast<Map<dynamic, dynamic>>()) {
    test('merge: ${vector['name']}', () {
      if (vector['effective'] case final Object expected) {
        expect(mergeProfiles(layers(vector)).view(), expected);
        return;
      }
      final expected = vector['error'] as Map;
      try {
        layers(vector);
        fail('accepted, and the vector refuses it');
      } on (int, ProfileException) catch (refusal) {
        final (index, error) = refusal;
        expect(index, expected['layer'], reason: 'the layer refused');
        expect(error.code, expected['code'], reason: '$error');
        if (error.code != 'parse') expect(error.key, expected['key']);
      }
    });
  }

  for (final vector in (vectors['sources'] as List).cast<Map<dynamic, dynamic>>()) {
    test('source: ${vector['source']}', () {
      final source = vector['source'] as String;
      if (vector['refused'] == true) {
        expect(() => OverlaySource.parseGit(source), throwsA(isA<ProfileException>()));
        return;
      }
      final parsed = OverlaySource.parseGit(source);
      expect((parsed.repo, parsed.reference, parsed.path), (vector['repo'], vector['ref'], vector['path']));
    });
  }

  for (final vector in (vectors['overlays'] as List).cast<Map<dynamic, dynamic>>()) {
    test('overlay: ${vector['name']}', () async {
      final os = vector['os'] as String;
      final layers = <String, Layer>{};
      final manifests = <String, Map<String, Object?>>{};
      final refused = <Map<String, Object?>>[];
      for (final overlay in (vector['overlays'] as List).cast<Map<dynamic, dynamic>>()) {
        final identity = overlay['identity'] as String;
        try {
          final layer = overlayLayer(
            identity: identity,
            root: overlay['root'] as String,
            os: os,
            harness: vector['harness'] as String?,
            pinned: overlay['pinned'] == true,
            tree: [
              for (final MapEntry(:key, :value) in (overlay['files'] as Map).entries)
                AosTreeEntry.file('$key', utf8.encode('$value')),
            ],
          );
          manifests[normalizeName(identity)] = layer.aos!.manifest;
          layers[normalizeName(identity)] = layer;
        } on OverlayRefusal catch (refusal) {
          refused.add({'identity': identity, 'code': refusal.code, 'subject': refusal.subject});
        }
      }
      expect(refused, vector['refused']);
      final (visited, _) = resolveRequires(manifests, manifests.keys.toList());
      final ordered = [
        for (final identity in compositionOrder(manifests, visited)) layers[identity]!,
        if (vector['user'] case final String user)
          Layer(const LayerId(LayerKind.user), Profile.parse(user, LayerKind.user)),
      ];
      final effective = mergeProfiles(ordered);
      expect(effective.view(), vector['view']);
      final host = (vector['host'] as Map?) ?? const {};
      try {
        final found = await effective.environment(
          lookup: (name) => host[name] as String?,
          readFile: (path, {within}) async => null,
        );
        expect(vector['environment'], isNotNull, reason: 'the environment builds, and the vector refuses it');
        expect({for (final MapEntry(:key, :value) in found.entries) key: value.expose()}, vector['environment']);
      } on ProfileException catch (error) {
        final expected = vector['environment_error'] as Map?;
        expect(expected, isNotNull, reason: 'refused: $error');
        expect(error.code, expected!['code']);
        expect(error.key, expected['key']);
      }
    });
  }

  test('the reserved variables are the vectors\' list', () {
    final reserved = vectors['reserved_env'] as Map;
    expect(reservedEnv, reserved['names']);
    expect(reservedEnvPrefixes, reserved['prefixes']);
    expect(reservedEnvSuffixes, reserved['suffixes']);
  });
}
