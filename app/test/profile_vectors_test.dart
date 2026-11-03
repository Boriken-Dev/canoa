// The profile vectors canoa-host is held to as well (crates/canoa-host/tests/profile.rs): each merge's view or first
// refusal, and each overlay source.
import 'dart:convert';
import 'dart:io';

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
}
